// Runs the generated WASM in Node and compares it with the native test
// vectors (crates/pqpq-sim/src/lib.rs, crates/pqpq-protocol/src/messages.rs).
// Usage: node web/check.mjs   (after building web/pkg)
import { readFile } from "node:fs/promises";
import { initSync, reference_state, reference_join_frame, validate, Client } from "./pkg/pqpq_web.js";

initSync({ module: await readFile(new URL("./pkg/pqpq_web_bg.wasm", import.meta.url)) });

const REFERENCE = [12.652196453530346, -43.00848023940093, 1.3063283155510665, 0.04017250679584599, -0.010879175534572292];
const JOIN_FRAME = [0, 0, 0, 28, 1, 0, 0, 0, 1, 0, 3, 102, 111, 111, 0, 4, 49, 50, 51, 52, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1];
const TOLERANCE = 1e-6;

const failures = [];
const state = reference_state();
REFERENCE.forEach((expected, i) => {
  if (!(Math.abs(state[i] - expected) < TOLERANCE)) failures.push(`sim[${i}] ${state[i]} != ${expected}`);
});
const frame = Array.from(reference_join_frame());
if (frame.join() !== JOIN_FRAME.join()) failures.push(`join frame ${frame} != ${JOIN_FRAME}`);
if (validate("foo", "1234") !== undefined) failures.push("valid input rejected");
if (validate("foo", "12 34") === undefined) failures.push("invalid room accepted");
const client = new Client();
client.join("foo", "1234");
if (Array.from(client.take_stream()).join() !== JOIN_FRAME.join()) failures.push("client join frame differs");

if (failures.length) {
  console.error("NG\n" + failures.join("\n"));
  process.exit(1);
}
console.log("OK: WASM simulation and codec match the native test vectors");
