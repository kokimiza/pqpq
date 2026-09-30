// Real Chromium + production Native QUIC acceptance test. Test-only driving
// sends keyboard events; it never sends positions or finishes to the server.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { mkdtemp, readFile, mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { createServer } from "node:net";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { udpLink } from "./udp-link.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const require = createRequire(new URL("../web/package.json", import.meta.url));
const { chromium } = require("playwright");
const ext = process.platform === "win32" ? ".exe" : "";
const impaired = process.argv.includes("--impaired");
const build = join(root, "target/verify");
const env = { ...process.env, CARGO_TARGET_DIR: build };
function run(command, args, capture = false) {
  const r = spawnSync(command, args, { cwd: root, env, encoding: "utf8", windowsHide: true, stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit" });
  if (r.error) throw r.error;
  if (r.status !== 0) {
    if (capture) for (const line of (r.stdout ?? "").split("\n")) {
      try { const event = JSON.parse(line); if (event.message?.rendered) console.error(event.message.rendered); } catch {}
    }
    throw new Error(`${command} exited ${r.status}`);
  }
  return r.stdout;
}
async function freePort() {
  const s = createServer();
  await new Promise((resolve, reject) => { s.on("error", reject); s.listen(0, "127.0.0.1", resolve); });
  const port = s.address().port;
  await new Promise((resolve) => s.close(resolve));
  return port;
}
async function until(predicate, timeout = 10000) {
  const start = Date.now();
  while (!predicate()) {
    if (Date.now() - start > timeout) throw new Error("test deadline exceeded");
    await new Promise((r) => setTimeout(r, 50));
  }
}

// Build all targets together so Cargo keeps one set of native dependency
// features (quiche's TLS toolchain is expensive to rebuild per package).
const artifacts = run("cargo", ["build", "--workspace", "--all-targets", "--locked", "--message-format=json"], true);
const nativeExe = artifacts.split("\n").filter(Boolean).map((s) => JSON.parse(s)).find((a) => a.executable && a.target.name === "network_race").executable;
await mkdir(join(root, "target/crossplay"), { recursive: true });
const evidence = await mkdtemp(join(root, "target/crossplay/run-"));
const certDir = join(evidence, "certs");
run(join(build, `debug/examples/dev_cert${ext}`), [certDir]);
const certFile = join(certDir, "localhost.pem");
const cert = new X509Certificate(await readFile(certFile));
const pin = createHash("sha256").update(cert.publicKey.export({ type: "spki", format: "der" })).digest("base64");
const nativePort = await freePort();
const webPort = await freePort();
const webBackendPort = impaired ? await freePort() : webPort;
const links = impaired ? [await udpLink(nativePort), await udpLink(webBackendPort, webPort)] : [];
const origin = `https://127.0.0.1:${webPort}`;
const room = `crossplay-${Date.now()}`;
const settings = { ...env, PQPQ_BIND_ADDR: `127.0.0.1:${nativePort}`, PQPQ_SERVER_ADDR: `127.0.0.1:${links[0]?.port ?? nativePort}`, PQPQ_TLS_SERVER_NAME: "localhost", PQPQ_CA_FILE: certFile,
  PQPQ_WEBTRANSPORT_ADDR: `127.0.0.1:${webBackendPort}`, PQPQ_HTTPS_ADDR: `127.0.0.1:${webPort}`, PQPQ_WEB_ORIGIN: origin,
  PQPQ_WEB_ROOT: join(root, "web"), PQPQ_TLS_CERT: certFile, PQPQ_TLS_KEY: join(certDir, "localhost-key.pem"), PQPQ_PIN_WEB_CERT: "true", PQPQ_TEST_ROOM: room };
const server = spawn(join(build, `debug/pqpq-server${ext}`), [], { cwd: root, env: settings, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
let serverLog = "";
server.stderr.on("data", (b) => { serverLog += b; });
let browser, native, page;
let nativeLog = "";
const pageErrors = [];
try {
  await until(() => serverLog.includes("https tcp") || server.exitCode !== null);
  assert.equal(server.exitCode, null, serverLog);
  // Pin just this test certificate in an isolated browser, without installing
  // a root CA or disabling certificate validation for arbitrary hosts.
  browser = await chromium.launch({ headless: true, args: [`--ignore-certificate-errors-spki-list=${pin}`] });
  const context = await browser.newContext();
  page = await context.newPage();
  page.on("pageerror", (e) => pageErrors.push(e.message));
  page.on("console", (m) => { if (m.type() === "error") console.error("Browser:", m.text()); });
  await page.goto(`${origin}/#room=${room}`);
  await page.locator("#join-button").waitFor({ state: "visible" });
  await page.locator("#name").fill("same-name");
  assert.equal(await page.locator("#room").inputValue(), room);
  // Cancel during the first handshake and immediately retry. A stale ready
  // rejection must not clear or overwrite the new session's UI.
  await page.evaluate(() => {
    document.querySelector("#join-form").requestSubmit();
    dispatchEvent(new KeyboardEvent("keydown", { code: "KeyQ" }));
    document.querySelector("#join-form").requestSubmit();
  });
  await page.locator("#lobby").waitFor({ state: "visible" });
  console.log("Browser joined over WebTransport");

  native = spawn(nativeExe, ["--ignored", "--nocapture", "native_three_laps"], { cwd: root, env: settings, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
  native.stdout.on("data", (b) => { nativeLog += b; });
  native.stderr.on("data", (b) => { nativeLog += b; });
  await until(() => nativeLog.includes("NATIVE_JOINED") || native.exitCode !== null);
  assert.equal(native.exitCode, null, nativeLog);
  await page.waitForFunction(() => document.querySelectorAll("#roster li").length === 2);
  assert.match(await page.locator("#roster").innerText(), /same-name/);

  // Observe the already-rendered client view, then inject the same keyboard
  // events a human uses. This helper exists only in the test page.
  await page.evaluate(async () => {
    const { Client, course_geometry } = await import("/pkg/pqpq_web.js");
    const g = course_geometry();
    const points = [];
    for (let i = 0; i < g.left.length; i += 2) points.push([(g.left[i] + g.right[i]) / 2, (g.left[i + 1] + g.right[i + 1]) / 2]);
    const lengths = points.map((p, i) => Math.hypot(points[(i + 1) % points.length][0] - p[0], points[(i + 1) % points.length][1] - p[1]));
    const held = new Set();
    const key = (code, on) => {
      if (held.has(code) === on) return;
      on ? held.add(code) : held.delete(code);
      dispatchEvent(new KeyboardEvent(on ? "keydown" : "keyup", { code, bubbles: true }));
    };
    const original = Client.prototype.cars;
    const driver = globalThis.testDriver = { view: null, enabled: true };
    Client.prototype.cars = function(now) {
      const cars = original.call(this, now);
      const v = driver.view = this.view(now);
      if (!driver.enabled) return cars;
      if (v.screen !== "race" || v.finished) {
        for (const code of [...held]) key(code, false);
        return cars;
      }
      for (let i = 0; i < cars.length; i += 5) {
        if (!cars[i + 4]) continue;
        const [, x, y, direction] = cars.subarray(i, i + 5);
        let best = { distance: Infinity };
        for (let j = 0; j < points.length; j++) {
          const [ax, ay] = points[j], [bx, by] = points[(j + 1) % points.length];
          const t = Math.max(0, Math.min(1, ((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / lengths[j] ** 2));
          const distance = Math.hypot(x - (ax + t * (bx - ax)), y - (ay + t * (by - ay)));
          if (distance < best.distance) best = { distance, j, t };
        }
        let segment = best.j, along = best.t * lengths[segment] + 15;
        while (along > lengths[segment]) { along -= lengths[segment]; segment = (segment + 1) % points.length; }
        const a = points[segment], b = points[(segment + 1) % points.length], t = along / lengths[segment];
        const desired = Math.atan2(a[1] + t * (b[1] - a[1]) - y, a[0] + t * (b[0] - a[0]) - x);
        const diff = Math.atan2(Math.sin(desired - direction), Math.cos(desired - direction));
        key("KeyW", (v.speedKmh ?? 0) < 100);
        key("KeyA", diff > 0.05);
        key("KeyD", diff < -0.05);
      }
      return cars;
    };
  });
  await page.locator("#ready").click();
  await page.waitForFunction(() => document.querySelector("#hud").textContent.includes("LAP"));
  console.log("Mixed race started: same display name, different PlayerIds");
  const spectator = await context.newPage();
  spectator.on("pageerror", (e) => pageErrors.push(e.message));
  await spectator.goto(`${origin}/#room=${room}`);
  await spectator.locator("#name").fill("same-name");
  await spectator.locator("#join-button").click();
  await spectator.locator("#race").waitFor({ state: "visible" });
  assert.match(await spectator.locator("#hud").innerText(), /観戦|SPECTAT/i);
  await spectator.locator("#race .leave").click();
  await spectator.close();
  await page.screenshot({ path: join(evidence, "race.png") });
  const progress = setInterval(async () => {
    try { console.log(await page.locator("#hud").innerText()); } catch {}
  }, 15000);
  try { await page.locator("#results").waitFor({ state: "visible", timeout: 120000 }); }
  finally { clearInterval(progress); }
  const results = await page.evaluate(() => testDriver.view.results);
  assert.equal(results.length, 2);
  assert.ok(results.every((r) => r.status === "FIN" && r.laps === 3));
  await until(() => native.exitCode !== null);
  assert.equal(native.exitCode, 0, nativeLog);
  const times = nativeLog.match(/NATIVE_RESULTS ([\d:,]+)/)[1].split(",").map((r) => Number(r.split(":")[1]));
  assert.deepEqual(results.map((r) => r.timeMs), times);
  await page.screenshot({ path: join(evidence, "results.png") });
  console.log("Both clients finished 3 laps; authoritative results match", times);

  await page.evaluate(() => { testDriver.enabled = false; });
  await page.locator("#results .leave").click();
  await page.locator("#join").waitFor({ state: "visible" });
  // Same room must be recreated as an empty lobby, and repeated joins must
  // not retain animation loops, old transports or WASM client instances.
  for (let i = 0; i < 3; i++) {
    await page.locator("#join-button").click();
    await page.locator("#lobby").waitFor({ state: "visible" });
    assert.equal(await page.locator("#roster li").count(), 1);
    await page.locator("#lobby .leave").click();
    await page.locator("#join").waitFor({ state: "visible" });
  }
  const unsupported = await context.newPage();
  await unsupported.addInitScript(() => { delete globalThis.WebTransport; });
  await unsupported.goto(origin);
  await unsupported.locator("#error").waitFor({ state: "visible" });
  assert.match(await unsupported.locator("#error-message").innerText(), /WebTransport/);
  await unsupported.close();
  const missingWasm = await context.newPage();
  await missingWasm.route("**/pkg/pqpq_web_bg.wasm", (route) => route.fulfill({ status: 404, body: "missing" }));
  await missingWasm.goto(origin);
  await missingWasm.locator("#error").waitFor({ state: "visible" });
  assert.match(await missingWasm.locator("#error-message").innerText(), /WASM/);
  await missingWasm.close();
  assert.deepEqual(pageErrors, []);
  await writeFile(join(evidence, "result.json"), JSON.stringify({ impaired, links: links.map((l) => l.stats), results, nativeLog, pageErrors }, null, 2));
  console.log(`PASS: crossplay, results, empty-room deletion and repeated rejoin. Evidence: ${evidence}`);
} catch (e) {
  if (page) {
    console.error(await page.locator("body").innerText().catch(() => "page unavailable"));
    await page.screenshot({ path: join(evidence, "failure.png") }).catch(() => {});
  }
  console.error(serverLog, nativeLog);
  throw e;
} finally {
  if (native && native.exitCode === null) native.kill();
  await browser?.close();
  server.kill();
  for (const link of links) link.close();
  await writeFile(join(evidence, "server.log"), serverLog);
}
