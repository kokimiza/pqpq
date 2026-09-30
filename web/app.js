// Thin browser layer: DOM, Canvas, keyboard and WebTransport I/O only.
// Codec, prediction, reconciliation and interpolation live in pqpq-web (WASM).
let Client, course_geometry, validate;

const $ = (id) => document.getElementById(id);
const SECTIONS = ["loading", "join", "connecting", "lobby", "race", "results", "error"];
// Display colors only; the same order as the terminal client's palette.
const PALETTE = ["#06b6d4", "#eab308", "#d946ef", "#22c55e", "#f87171", "#60a5fa", "#a8a29e", "#86efac"];
const KEYS = {
  ArrowUp: "up", KeyW: "up", ArrowDown: "down", KeyS: "down",
  ArrowLeft: "left", KeyA: "left", ArrowRight: "right", KeyD: "right",
};
const CONNECT_TIMEOUT_MS = 5000;
const MAX_CONTROL_BACKLOG = 256 * 1024;

let course = null;
let transportOptions = {};
/** Current connection: { client, transport, writer, datagrams, keys, closed } */
let session = null;
let connectionAttempt = 0;

function show(name) {
  for (const id of SECTIONS) $(id).hidden = id !== name;
}

function fatal(message) {
  $("error-message").textContent = message;
  show("error");
}

async function boot() {
  const missing = [];
  if (!globalThis.isSecureContext) missing.push("HTTPS");
  if (typeof WebAssembly !== "object") missing.push("WebAssembly");
  if (!("WebTransport" in globalThis)) missing.push("WebTransport");
  if (missing.length) {
    fatal(`このブラウザでは参加できません（${missing.join("・")} が利用できません）。`);
    return;
  }
  try {
    const wasm = await import("./pkg/pqpq_web.js");
    await wasm.default();
    ({ Client, course_geometry, validate } = wasm);
  } catch (error) {
    console.error(error);
    fatal("WASM を読み込めませんでした。");
    return;
  }
  try {
    const response = await fetch("./transport-config.json", { cache: "no-store" });
    if (!response.ok) throw new Error("transport configuration unavailable");
    const config = await response.json();
    if (config.certificateSha256 !== undefined) {
      const hash = config.certificateSha256;
      if (!Array.isArray(hash) || hash.length !== 32 || !hash.every((n) => Number.isInteger(n) && n >= 0 && n <= 255)) {
        throw new Error("invalid certificate hash");
      }
      transportOptions = { serverCertificateHashes: [{ algorithm: "sha-256", value: new Uint8Array(hash) }] };
    }
  } catch (error) {
    console.error(error);
    fatal("接続設定を読み込めませんでした。サーバの設定を確認してください。");
    return;
  }
  course = course_geometry();
  const room = new URLSearchParams(location.hash.slice(1)).get("room");
  if (room && validate("x", room) === undefined) $("room").value = room;
  $("join-button").disabled = false;
  show("join");
  $("name").focus();
}

$("join-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (!Client || session) return;
  const name = $("name").value;
  const room = $("room").value;
  const problem = validate(name, room);
  $("form-error").textContent = problem ?? "";
  if (problem) return;
  $("join-button").disabled = true;
  const attempt = ++connectionAttempt;
  try {
    await connect(name, room);
  } catch (error) {
    if (attempt !== connectionAttempt) return;
    console.error(error);
    session = null;
    show("join");
    $("form-error").textContent = "サーバに接続できませんでした（WebTransport / UDP を確認してください）。";
    $("join-button").disabled = false;
  }
});

async function connect(name, room) {
  $("connecting-room").textContent = room;
  show("connecting");
  const client = new Client();
  client.join(name, room);
  let transport;
  try { transport = new WebTransport(`https://${location.host}/transport`, transportOptions); }
  catch (error) { client.free(); throw error; }
  const s = {
    client,
    transport,
    writer: null,
    datagrams: null,
    keys: new Set(),
    closed: false,
    disposed: false,
    readers: [],
    frameId: null,
    timer: null,
    pendingBytes: 0,
    pendingDatagram: null,
    writingDatagram: false,
  };
  session = s;
  // Attach immediately: ready can reject before a stream exists.
  transport.closed.then(
    () => ended(s, "サーバとの接続が終了しました"),
    () => ended(s, "サーバとの接続が切れました"),
  );
  try {
    await deadline(transport.ready);
    if (session !== s || s.closed || transport.reliability === "reliable-only" || !transport.datagrams) {
      throw new Error("WebTransport datagrams are not available");
    }
    const stream = await deadline(transport.createBidirectionalStream());
    if (session !== s || s.closed || s.disposed) throw new Error("connection cancelled");
    s.writer = stream.writable.getWriter();
    s.datagrams = transport.datagrams.writable.getWriter();
    pump(s, stream.readable, (bytes) => client.on_stream(bytes, performance.now()));
    pump(s, transport.datagrams.readable, (bytes) => client.on_datagram(bytes, performance.now()));
    flush(s);
    // I/O and prediction do not depend on rendering. In particular, reply to
    // Ping from pump even when requestAnimationFrame is suspended in a tab.
    s.timer = setInterval(() => {
      if (s.closed || s.disposed) return;
      const k = s.keys;
      const on = (direction) => [...k].some((code) => KEYS[code] === direction);
      s.client.set_keys(on("up"), on("down"), on("left"), on("right"));
      s.client.update(performance.now());
      flush(s);
    }, 1000 / 30);
    s.frameId = requestAnimationFrame((now) => frame(s, now));
  } catch (error) {
    dispose(s);
    throw error;
  }
}

async function deadline(promise) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error("connection timeout")), CONNECT_TIMEOUT_MS); }),
    ]);
  } finally { clearTimeout(timer); }
}

async function pump(s, readable, handle) {
  const reader = readable.getReader();
  s.readers.push(reader);
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (s.disposed) return;
      if (done) {
        ended(s, "サーバとの通信ストリームが終了しました");
        s.transport.close();
        return;
      }
      handle(value);
      flush(s);
    }
  } catch {
    if (!s.disposed) {
      ended(s, "通信エラーが発生しました");
      s.transport.close();
    }
  } finally {
    reader.releaseLock();
  }
}

function ended(s, reason) {
  if (s.closed || s.disposed) return;
  s.closed = true;
  clearInterval(s.timer);
  s.client.closed(reason);
}

function flush(s) {
  if (s.closed || s.disposed || !s.writer) return;
  const bytes = s.client.take_stream();
  if (bytes.length) {
    s.pendingBytes += bytes.length;
    if (s.pendingBytes > MAX_CONTROL_BACKLOG) {
      ended(s, "通信が混雑しています");
      s.transport.close();
      return;
    }
    s.writer.write(bytes).catch(() => {
      ended(s, "制御メッセージを送信できませんでした");
      s.transport.close();
    }).finally(() => { s.pendingBytes -= bytes.length; });
  }
  for (let d = s.client.take_datagram(); d; d = s.client.take_datagram()) {
    s.pendingDatagram = d;
  }
  sendLatestInput(s);
}

function sendLatestInput(s) {
  if (s.closed || s.disposed || s.writingDatagram || !s.pendingDatagram) return;
  const bytes = s.pendingDatagram;
  s.pendingDatagram = null;
  s.writingDatagram = true;
  s.datagrams.write(bytes).catch(() => {
    ended(s, "操作を送信できませんでした");
    s.transport.close();
  }).finally(() => {
    s.writingDatagram = false;
    sendLatestInput(s);
  });
}

function frame(s, now) {
  if (session !== s || s.disposed) return;
  const view = s.client.view(now);
  render(view, s.client.cars(now));
  if (view.screen === "error") {
    if (!s.closed) {
      s.closed = true;
      clearInterval(s.timer);
      s.transport.close();
    }
    return;
  }
  s.frameId = requestAnimationFrame((time) => frame(s, time));
}

function dispose(s) {
  if (s.disposed) return;
  s.disposed = true;
  s.closed = true;
  clearInterval(s.timer);
  cancelAnimationFrame(s.frameId);
  for (const reader of s.readers) reader.cancel().catch(() => {});
  s.transport.close();
  s.client.free();
}

function leave() {
  const s = session;
  if (!s) return;
  ++connectionAttempt;
  s.client.leave();
  flush(s);
  session = null;
  clearInterval(s.timer);
  cancelAnimationFrame(s.frameId);
  setTimeout(() => dispose(s), 200);
  show("join");
  $("join-button").disabled = false;
}

// ---- input ----

function typing(target) {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target?.isContentEditable;
}

addEventListener("keydown", (event) => {
  if (!session || typing(event.target)) return;
  const key = KEYS[event.code];
  if (key) {
    session.keys.add(event.code);
    event.preventDefault();
  } else if (event.code === "KeyQ") {
    leave();
  } else if (event.code === "Enter" && !(event.target instanceof HTMLButtonElement)) {
    session.client.ready();
    flush(session);
    event.preventDefault();
  }
});

addEventListener("keyup", (event) => {
  const key = KEYS[event.code];
  if (session && key) session.keys.delete(event.code);
});

function neutral() {
  if (!session) return;
  session.keys.clear();
  session.client.neutral();
  flush(session);
}
addEventListener("blur", neutral);
document.addEventListener("visibilitychange", () => document.hidden && neutral());
// Best effort only; the server also notices the disconnect.
addEventListener("pagehide", () => {
  if (!session) return;
  session.client.leave();
  flush(session);
});

$("ready").addEventListener("click", () => {
  if (!session) return;
  session.client.ready();
  flush(session);
});
for (const button of document.querySelectorAll(".leave")) button.addEventListener("click", leave);
$("reload").addEventListener("click", () => location.reload());

// ---- rendering (textContent only: names are never parsed as HTML) ----

function setList(element, items) {
  const key = JSON.stringify(items);
  if (element.dataset.key === key) return;
  element.dataset.key = key;
  element.replaceChildren(...items.map(([text, me]) => {
    const li = document.createElement("li");
    li.textContent = text;
    if (me) li.className = "me";
    return li;
  }));
}

function clock(seconds) {
  const ms = Math.round(seconds * 1000);
  const pad = (n, w) => String(n).padStart(w, "0");
  return `${pad(Math.floor(ms / 60000), 2)}:${pad(Math.floor(ms / 1000) % 60, 2)}.${pad(ms % 1000, 3)}`;
}

function carLabel(index) {
  if (index < 0) return "?";
  let text = "";
  do {
    text = String.fromCharCode(65 + index % 26) + text;
    index = Math.floor(index / 26) - 1;
  } while (index >= 0);
  return text;
}

function render(v, cars) {
  for (const n of document.querySelectorAll(".notice")) n.textContent = v.notice ?? "";
  switch (v.screen) {
    case "connecting":
      show("connecting");
      return;
    case "error":
      fatal(v.error);
      return;
    case "lobby": {
      show("lobby");
      $("lobby-room").textContent = v.room;
      setList($("roster"), v.roster.map((e) => [
        `#${e.id}  ${e.ready ? "READY " : "未Ready"}  ${e.name}${e.me ? "（あなた）" : ""}`, e.me,
      ]));
      const racers = v.roster.filter((e) => e.racer).length;
      $("lobby-rule").textContent = `開始条件: ${v.minRacers}人以上・全員 READY（現在 ${racers}人）`;
      $("share").textContent = `${location.origin}/#room=${v.room}`;
      $("ready").disabled = v.roster.some((e) => e.me && e.ready);
      return;
    }
    case "results": {
      show("results");
      $("results-room").textContent = v.room;
      const body = $("result-rows");
      const key = JSON.stringify(v.results);
      if (body.dataset.key !== key) {
        body.dataset.key = key;
        body.replaceChildren(...v.results.map((r) => {
          const tr = document.createElement("tr");
          const time = r.status === "FIN" && r.timeMs !== undefined ? clock(r.timeMs / 1000) : `DNF（${r.laps}周）`;
          for (const text of [String(r.rank), r.label, r.name + (r.me ? "（あなた）" : ""), time]) {
            const td = document.createElement("td");
            td.textContent = text;
            tr.append(td);
          }
          return tr;
        }));
      }
      return;
    }
    default: {
      show("race");
      let hud = `ROOM ${v.room}`;
      if (v.screen === "countdown") hud += `   START IN ${(v.countdown ?? 0).toFixed(1)}`;
      if (v.screen === "spectate") hud += "   観戦中";
      if (v.rank !== undefined) hud += `   LAP ${v.finished ? "FIN" : `${v.lap}/${v.laps}`}   POS ${v.rank}/${v.entrants}`;
      if (v.rtt !== undefined) hud += `   RTT ${v.rtt}ms`;
      $("hud").textContent = hud;
      let hud2 = "";
      if (v.speedKmh !== undefined) hud2 += `SPEED ${v.speedKmh} km/h   `;
      if (v.time !== undefined) hud2 += `TIME ${clock(v.time)}   `;
      if (v.screen !== "spectate") hud2 += `YOU ${v.meLabel}`;
      if (v.stale) hud2 += "   通信遅延";
      $("hud2").textContent = hud2;
      setList($("standings"), v.standings.map((c) => [
        `${c.label} ${c.name.slice(0, 12)} ${c.status === "RUN" ? `L${c.lap}` : c.status}`, c.me,
      ]));
      draw(cars);
    }
  }
}

function draw(cars) {
  const canvas = $("track");
  const ratio = devicePixelRatio || 1;
  const w = Math.round(canvas.clientWidth * ratio);
  const h = Math.round(canvas.clientHeight * ratio);
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  const ctx = canvas.getContext("2d");
  const css = getComputedStyle(document.documentElement);
  const [minX, minY, maxX, maxY] = course.bounds;
  const pad = 12 * ratio;
  const scale = Math.min((w - 2 * pad) / (maxX - minX), (h - 2 * pad) / (maxY - minY));
  const ox = (w - (maxX - minX) * scale) / 2;
  const oy = (h - (maxY - minY) * scale) / 2;
  const x = (wx) => ox + (wx - minX) * scale;
  const y = (wy) => h - (oy + (wy - minY) * scale); // world y points up

  ctx.clearRect(0, 0, w, h);
  const loop = (pts) => {
    ctx.moveTo(x(pts[0]), y(pts[1]));
    for (let i = 2; i < pts.length; i += 2) ctx.lineTo(x(pts[i]), y(pts[i + 1]));
    ctx.closePath();
  };
  ctx.beginPath();
  loop(course.left);
  loop(course.right);
  ctx.fillStyle = css.getPropertyValue("--track");
  ctx.fill("evenodd");
  ctx.strokeStyle = css.getPropertyValue("--edge");
  ctx.lineWidth = 1.5 * ratio;
  ctx.stroke();
  const f = course.finish;
  ctx.beginPath();
  ctx.moveTo(x(f[0]), y(f[1]));
  ctx.lineTo(x(f[2]), y(f[3]));
  ctx.strokeStyle = css.getPropertyValue("--fg");
  ctx.lineWidth = 3 * ratio;
  ctx.stroke();

  const radius = Math.max(course.carRadius * scale, 5 * ratio);
  ctx.font = `700 ${Math.round(11 * ratio)}px ui-monospace, monospace`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  for (let i = 0; i < cars.length; i += 5) {
    const [label, wx, wy, dir, me] = cars.subarray(i, i + 5);
    const cx = x(wx);
    const cy = y(wy);
    ctx.beginPath();
    ctx.arc(cx, cy, radius, 0, Math.PI * 2);
    ctx.fillStyle = label >= 0 ? PALETTE[label % PALETTE.length] : "#888";
    ctx.fill();
    if (me) {
      ctx.lineWidth = 2.5 * ratio;
      ctx.strokeStyle = css.getPropertyValue("--fg");
      ctx.stroke();
    }
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(cx + Math.cos(dir) * radius * 1.8, cy - Math.sin(dir) * radius * 1.8);
    ctx.lineWidth = 2 * ratio;
    ctx.strokeStyle = css.getPropertyValue("--fg");
    ctx.stroke();
    ctx.fillStyle = css.getPropertyValue("--fg");
    ctx.fillText(carLabel(label), cx, cy - radius - 8 * ratio);
  }
}

boot();
