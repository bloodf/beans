import { createHash } from "node:crypto";
import { readFile, writeFile, realpath } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { execFileSync } from "node:child_process";
import assert from "node:assert/strict";

export const exampleCommit = "35780590139c9e0b09f3f8e6ea87a1f1e6524db1";
export const fixtureCommit = "57566a4b252c789eba0f7f22cefd47239d439026";
const examplePath = "crates/agent/examples/output_baseline.rs";
const fixturePath = "crates/agent/fixtures/output-baseline";
const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const git = (repo: string, ...args: string[]) => execFileSync("git", ["-C", repo, ...args]);
const text = (result: any): string => {
  assert(Array.isArray(result.content), "missing ToolResult content");
  return result.content.filter((part: any) => part.type === "text").map((part: any) => {
    assert.equal(typeof part.text, "string");
    return part.text;
  }).join("\n");
};
const counts = (value: string) => ({ utf8_bytes: Buffer.byteLength(value), unicode_scalars: Array.from(value).length });

// Consume receipts produced by the real, independently leased Rust example. Never rebuild it here.
export async function checkOutputBaseline(receipt: string, repo: string, binary: string) {
  assert(isAbsolute(receipt) && isAbsolute(repo) && isAbsolute(binary), "paths must be absolute");
  const head = git(repo, "rev-parse", "HEAD").toString().trim();
  assert.equal(head, exampleCommit, "example checkout must be pinned, not current production tree");
  const original = git(repo, "show", `${exampleCommit}:${examplePath}`);
  assert.deepEqual(await readFile(join(repo, examplePath)), original, "example differs from immutable candidate");
  assert.equal(git(repo, "status", "--porcelain", "--untracked-files=no").length, 0, "example checkout has tracked modifications");
  const json = async (name: string) => JSON.parse(await readFile(join(receipt, name), "utf8"));
  const provenance = await json("provenance.json");
  assert.equal(provenance.checkout_head, exampleCommit);
  assert.equal(provenance.fixture_commit, fixtureCommit);
  assert.equal(await realpath(provenance.executable), await realpath(binary), "receipt binary differs from supplied executable");
  const binaryHash = sha256(await readFile(binary));
  const stdout = await readFile(join(receipt, "stdout.txt"), "utf8");
  assert.equal(stdout.trim(), "PASS: nonzero exits, exact script ingress, final emitted notices, read continuation, listing recovery");
  assert.equal((await readFile(join(receipt, "stderr.txt"))).length, 0, "worker stderr is not empty");
  const measurements = await json("measurements.json");
  assert(Array.isArray(measurements.measurements) && Array.isArray(measurements.nested_ingress));
  const rows = measurements.nested_ingress;
  const identities = new Set<string>();
  for (const row of rows) {
    assert.equal(typeof row.id, "string");
    assert.equal(typeof row.tool, "string");
    assert(!identities.has(`${row.tool}:${row.id}`), "duplicate Recorder identity");
    identities.add(`${row.tool}:${row.id}`);
    assert.match(row.result_path, /^nested-\d+\.json$/);
    assert.equal(row.blocked, false);
    const result = await json(row.result_path);
    const ingress = row.tool === "bash" ? result.structured : text(result);
    assert.equal(row.ingress_json_bytes, Buffer.byteLength(JSON.stringify(ingress)));
    assert.equal(row.ingress_text_bytes, typeof ingress === "string" ? Buffer.byteLength(ingress) : null);
    assert.equal(row.is_error, result.is_error === true);
  }
  const nested = async (result: any, tool: string) => {
    const calls = result.details.calls.filter((call: any) => call.name === tool);
    assert.equal(calls.length, 1, `ambiguous ${tool} call`);
    const matches = rows.filter((row: any) => row.tool === tool && row.id === calls[0].id);
    assert.equal(matches.length, 1, `missing Recorder ${tool}/id`);
    return { row: matches[0], result: await json(matches[0].result_path) };
  };
  const fixture = (name: string) => git(repo, "show", `${fixtureCommit}:${fixturePath}/${name}`);
  const spill = async (label: string) => {
    const metadata = await json(`${label}-spill.json`);
    assert.equal(metadata.kind, label);
    assert.equal(metadata.durable_relative_path, `${label}-full-output.bin`);
    const bytes = await readFile(join(receipt, metadata.durable_relative_path));
    assert.equal(bytes.length, metadata.bytes);
    assert.equal(sha256(bytes), metadata.sha256, `altered ${label} spill`);
    return bytes;
  };
  const metrics: any[] = [];
  for (const [label, name, exit] of [["build", "failing-build.log", 1], ["tests", "test-output.log", 101]] as const) {
    const raw = fixture(name);
    const direct = await json(`${label}.json`);
    const final = await json(`${label}-codemode.json`);
    assert.equal(direct.is_error, true);
    assert.equal(direct.details.exit_code, exit);
    assert(text(direct).includes(`Command exited with code ${exit}`));
    assert.deepEqual(await spill(label), raw, "direct spill loses exact subprocess bytes");
    assert.deepEqual(await readFile(join(receipt, `${label}.txt`)), Buffer.from(text(direct)));
    const ingress = await nested(final, "bash");
    assert.equal(ingress.row.is_error, true);
    assert.equal(ingress.result.structured.exit_code, exit);
    assert.equal(ingress.result.structured.truncated, false);
    assert.deepEqual(Buffer.from(ingress.result.structured.output), raw, "Recorder ingress differs from fixture");
    assert.equal(ingress.row.bash_output_bytes, raw.length);
    assert.equal(ingress.row.exit_code, exit);
    assert.notEqual(final.is_error, true);
    assert(text(final).startsWith("Script completed\n") && text(final).includes("Warning: truncated output"));
    assert.deepEqual(await readFile(join(receipt, `${label}-codemode.txt`)), Buffer.from(text(final)));
    const full = await spill(`${label}-codemode`);
    assert(full.includes(raw), "codemode spill loses emitted log");
    const measurement = measurements.measurements.filter((item: any) => item.case === label);
    assert.equal(measurement.length, 1);
    assert.equal(measurement[0].raw_bytes, raw.length);
    assert.equal(measurement[0].direct_model_text_bytes, Buffer.byteLength(text(direct)));
    assert.equal(measurement[0].script_bash_ingress_output_bytes, raw.length);
    assert.equal(measurement[0].codemode_final_model_text_bytes, Buffer.byteLength(text(final)));
    assert.equal(measurement[0].exit_code, exit);
    metrics.push({ case: label, exit_code: exit, raw: counts(raw.toString("utf8")), direct_model_text: counts(text(direct)), script_ingress: counts(ingress.result.structured.output), final_model_text: counts(text(final)), direct_spill_sha256: sha256(raw), codemode_spill_sha256: sha256(full) });
  }
  const pages = measurements.measurements.filter((item: any) => item.case === "read");
  assert(pages.length > 1, "read recovery not exercised");
  let recovered = "";
  let offset = 1;
  for (let index = 0; index < pages.length; index++) {
    const result = await json(`read-${index}.json`);
    assert.notEqual(result.is_error, true);
    assert.equal(pages[index].offset, offset);
    const value = text(result);
    assert.equal(pages[index].final_model_text_bytes, Buffer.byteLength(value));
    // Follow the emitted continuation, not a copied truncation algorithm.
    const marker = value.lastIndexOf("\n\n[Showing lines ");
    const body = marker < 0 ? value : value.slice(0, marker);
    recovered += (index ? "\n" : "") + body;
    const next = marker < 0 ? null : Number(value.slice(marker).match(/Use offset=(\d+) /)?.[1]);
    assert.equal(pages[index].next_offset, next);
    if (index < pages.length - 1) { assert(next !== null && next > offset); offset = next; }
    else assert.equal(next, null);
  }
  assert.deepEqual(Buffer.from(recovered), fixture("large-response.json"));
  JSON.parse(recovered);
  const entries = JSON.parse(fixture("directory-entries.json").toString()).entries;
  const expected = entries.map((entry: any) => entry.name + (entry.kind === "directory" ? "/" : "")).sort((a: string, b: string) => a.toLowerCase() < b.toLowerCase() ? -1 : a.toLowerCase() > b.toLowerCase() ? 1 : 0).join("\n");
  const prefix = await json("ls-prefix.json");
  const listing = await json("ls-recovered.json");
  assert.notEqual(prefix.is_error, true);
  assert(text(prefix).includes("500 entries limit reached"));
  assert.notEqual(listing.is_error, true);
  assert.equal(text(listing), expected);
  const bridge = await json("read-ls-codemode.json");
  assert.notEqual(bridge.is_error, true);
  const readBridge = await nested(bridge, "read");
  const lsBridge = await nested(bridge, "ls");
  assert.equal(text(readBridge.result), text(await json("read-0.json")));
  assert.equal(text(lsBridge.result), expected);
  await spill("read-ls-codemode");
  metrics.push({ case: "read", pages: pages.length, recovered: counts(recovered) }, { case: "ls", entries: entries.length, recovered: counts(expected) }, { case: "read-ls-codemode", final_model_text: counts(text(bridge)) });
  return { schema: 1, example_commit: exampleCommit, fixture_commit: fixtureCommit, executable_sha256: binaryHash, metrics, scope: "Provider-free runtime mechanics only; Unicode scalars and UTF-8 bytes are not provider tokens or quality scores." };
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const run = args[0] === "--run";
  if (run) args.shift();
  const [receipt, repo, binary] = args;
  if (!receipt || !repo || !binary || args.length !== 3 || !args.every(isAbsolute)) throw new Error("usage: bun run scripts/check-output-baseline.ts [--run] /absolute/actual-receipt /absolute/pinned-example-checkout /absolute/leased-example-binary");
  if (run) {
    assert.equal(git(repo, "rev-parse", "HEAD").toString().trim(), exampleCommit);
    assert.deepEqual(await readFile(join(repo, examplePath)), git(repo, "show", `${exampleCommit}:${examplePath}`));
    assert.equal(git(repo, "status", "--porcelain", "--untracked-files=no").length, 0);
    // --run requires an externally allocated runtime lease. No Cargo or implicit build.
    execFileSync(binary, [receipt], { cwd: repo, stdio: "inherit", env: { PATH: "/usr/bin:/bin", HOME: "/nonexistent", TMPDIR: process.env.TMPDIR ?? "/tmp" } });
  }
  const report = await checkOutputBaseline(receipt, repo, binary);
  await writeFile(join(receipt, "runner-metrics.json"), JSON.stringify(report, null, 2) + "\n", { flag: "wx", mode: 0o600 });
  console.log(JSON.stringify(report, null, 2));
}
