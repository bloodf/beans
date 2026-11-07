// Read-only tracked path/content check. Supply retired project names as arguments.
// README records provenance; the enumerated legal notices retain foreign attribution.
// Run only after the integration owner reconciles tracked path moves.
import { spawn } from "node:child_process";
import { lstat, readFile, readlink } from "node:fs/promises";
import { join } from "node:path";

const ROOT = join(import.meta.dirname, "..");
const retired = process.argv.slice(2).map((name) => name.toLowerCase());
if (!retired.length || retired.some((name) => !name || name.trim() !== name)) {
  throw new Error("usage: pnpm run scripts/check-tracked-names.ts <retired-name>...");
}
const legalNotices: Record<string, true> = {
  LICENSE: true,
  "packages/beans-blobatar/LICENSE": true,
  "web/public/providers/LICENSE": true,
};

const { stdout, stderr, status } = await new Promise<{ stdout: string; stderr: string; status: number | null }>((resolve, reject) => {
  const child = spawn("git", ["ls-files", "-z", "--cached"], {
    cwd: ROOT,
    stdio: ["ignore", "pipe", "pipe"],
  });
  const stdoutChunks: Buffer[] = [];
  const stderrChunks: Buffer[] = [];
  child.stdout.on("data", (chunk) => stdoutChunks.push(chunk));
  child.stderr.on("data", (chunk) => stderrChunks.push(chunk));
  child.on("error", (err) => reject(err));
  child.on("close", (code, signal) => {
    if (signal) {
      reject(new Error(`git ls-files killed by signal ${signal}`));
    } else {
      resolve({
        stdout: Buffer.concat(stdoutChunks).toString("utf8"),
        stderr: Buffer.concat(stderrChunks).toString("utf8"),
        status: code,
      });
    }
  });
});

if (status !== 0) throw new Error(`Cannot read tracked inventory: ${stderr}`);
const paths = [...new Set(stdout.split("\0").filter(Boolean))].sort();
const problems: string[] = [];
for (const path of paths) {
  const lower = path.toLowerCase();
  const named = retired.find((name) => lower.includes(name));
  if (named) problems.push(`${path}: retired project name in tracked path (${named})`);
  if (path === "README.md" || legalNotices[path] === true) continue;
  try {
    const file = join(ROOT, path);
    const info = await lstat(file);
    if (!info.isFile() && !info.isSymbolicLink()) {
      problems.push(`${path}: tracked content is not a regular file or symbolic link`);
      continue;
    }
    // Scan link metadata without following it into untracked or private data.
    // ASCII identifiers/URLs remain detectable in regular binary metadata too.
    const text = info.isSymbolicLink() ? await readlink(file) : (await readFile(file)).toString("utf8");
    for (const [index, line] of text.split("\n").entries()) {
      const name = retired.find((name) => line.toLowerCase().includes(name));
      if (name) problems.push(`${path}:${index + 1}: retired project name in tracked content (${name})`);
    }
  } catch (error) {
    problems.push(`${path}: cannot inspect tracked content (${error instanceof Error ? error.message : error})`);
  }
}
if (problems.length) {
  for (const problem of problems) console.error(problem);
  process.exit(1);
}
console.log(`Inspected ${paths.length} tracked paths; no retired names outside README and enumerated legal notices.`);
