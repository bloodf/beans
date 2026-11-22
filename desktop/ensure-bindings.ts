// The page imports the Go host's generated bindings from `src/mygo.ts`, which `mygo generate`
// writes and git does not keep. On a fresh checkout `typecheck` and the tests would fail on the
// missing module (#569), so they make the bindings first when they are missing. That needs Go.
import { existsSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL(".", import.meta.url));
const bindings = fileURLToPath(new URL("./src/mygo.ts", import.meta.url));

export function ensureBindings(): boolean {
  if (existsSync(bindings)) return true;
  console.log("src/mygo.ts is missing: generating the Go host's bindings with `mygo generate`…");
  const run = spawnSync("pnpm", ["exec", "mygo", "generate"], { cwd: root, stdio: "inherit", shell: process.platform === "win32" });
  if (run.status === 0 && existsSync(bindings)) return true;
  console.error(
    "Could not generate src/mygo.ts. It needs the Go toolchain (see desktop/go.mod) on PATH: install Go, then run `pnpm --dir desktop run generate`.",
  );
  return false;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  if (!ensureBindings()) process.exit(1);
}
