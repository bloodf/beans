import { test, expect } from "bun:test";
import { cp, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkOutputBaseline, exampleCommit, fixtureCommit } from "./check-output-baseline.ts";

// Run after Main leases the actual Rust example; no synthesized ToolResult or mock subprocess.
const receipt = process.env.OUTPUT_BASELINE_RECEIPT;
const checkout = process.env.OUTPUT_BASELINE_CHECKOUT;
const binary = process.env.OUTPUT_BASELINE_BINARY;
test("rejects unpinned checkout; verifies real receipts when supplied", async () => {
  await expect(checkOutputBaseline("/unused-receipt", import.meta.dir + "/..", "/unused-binary")).rejects.toThrow("example checkout must be pinned");
  if (!receipt || !checkout || !binary) {
    console.log("Runtime receipt checks NOT RUN: supply OUTPUT_BASELINE_RECEIPT, OUTPUT_BASELINE_CHECKOUT and OUTPUT_BASELINE_BINARY after central lease.");
    return;
  }
  const report = await checkOutputBaseline(receipt!, checkout!, binary!);
  expect(report.example_commit).toBe(exampleCommit);
  expect(report.fixture_commit).toBe(fixtureCommit);
  expect(report.metrics.filter(row => row.exit_code !== undefined).map(row => row.exit_code)).toEqual([1, 101]);
  const temporary = await mkdtemp(join(tmpdir(), "beans-output-runner-test-"));
  try {
    await cp(receipt!, temporary, { recursive: true });
    const path = join(temporary, "build-full-output.bin");
    const original = await readFile(path);
    const changed = Buffer.from(original);
    changed[0] ^= 1;
    await writeFile(path, changed);
    await expect(checkOutputBaseline(temporary, checkout!, binary!)).rejects.toThrow("altered build spill");
    await writeFile(path, original);
    const provenancePath = join(temporary, "provenance.json");
    const provenance = JSON.parse(await readFile(provenancePath, "utf8"));
    provenance.checkout_head = "dc74a8cb37e98cc1db72a3b61cf78ead2d79cec9";
    await writeFile(provenancePath, JSON.stringify(provenance));
    await expect(checkOutputBaseline(temporary, checkout!, binary!)).rejects.toThrow();
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});
