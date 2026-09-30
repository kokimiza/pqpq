import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { existsSync } from "node:fs";
import { join } from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const executable = process.platform === "win32" ? "wasm-bindgen.exe" : "wasm-bindgen";
const local = join(root, "target", "wasm-tools", "bin", executable);
function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit", windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
run("cargo", ["build", "-p", "pqpq-web", "--target", "wasm32-unknown-unknown", "--release", "--locked"]);
run(existsSync(local) ? local : executable, ["target/wasm32-unknown-unknown/release/pqpq_web.wasm", "--target", "web", "--out-dir", "web/pkg"]);
