// Test-only UDP link. Applies seeded delay, jitter, loss and reordering to
// encrypted QUIC packets in both directions, without inspecting game data.
import { createSocket } from "node:dgram";

export async function udpLink(targetPort, port = 0) {
  const socket = createSocket("udp4");
  const peers = new Map(), timers = new Set();
  const stats = { packets: 0, dropped: 0, delivered: 0 };
  let seed = 0x50515051, closed = false;
  const random = () => ((seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0) / 2 ** 32);
  function forward(output, packet, remotePort, address) {
    stats.packets++;
    if (random() < 0.05 || timers.size >= 4096) { stats.dropped++; return; }
    // 25 +/- 20 ms each way; an extra 50 ms on 10% reverses packet order.
    const delay = 5 + random() * 40 + (random() < 0.1 ? 50 : 0);
    const timer = setTimeout(() => {
      timers.delete(timer);
      if (!closed) output.send(packet, remotePort, address, (error) => { if (!error) stats.delivered++; });
    }, delay);
    timers.add(timer);
  }
  socket.on("message", (packet, peer) => {
    const key = `${peer.address}:${peer.port}`;
    let upstream = peers.get(key);
    if (!upstream) {
      if (peers.size >= 128) return;
      upstream = createSocket("udp4");
      upstream.on("error", () => {});
      upstream.on("message", (reply) => forward(socket, reply, peer.port, peer.address));
      peers.set(key, upstream);
    }
    forward(upstream, packet, targetPort, "127.0.0.1");
  });
  await new Promise((resolve, reject) => { socket.once("error", reject); socket.bind(port, "127.0.0.1", resolve); });
  return {
    port: socket.address().port, stats,
    close() {
      closed = true;
      for (const timer of timers) clearTimeout(timer);
      for (const peer of peers.values()) { try { peer.close(); } catch {} }
      socket.close();
    },
  };
}
