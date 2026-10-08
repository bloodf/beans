import { expect, test } from "bun:test";
import { join } from "node:path";

// prost-build, reached through lancedb in the Runner build, runs `protoc` from a build script and
// does not bundle it. Every job that compiles the CLI must install a pinned, checksummed protoc
// before its first cargo (or cargo-driving script) step. The phone core and the markdown crate
// do not reach prost-build, so the macOS app and EAS jobs are outside this contract.
const ROOT = join(import.meta.dir, "..");
const PINNED = /^protoc@\d+\.\d+\.\d+$/;
const ACTION = /^taiki-e\/install-action@[0-9a-f]{40}$/;
const COMPILES = /\bcargo\b|release-mac|desktop\.ts release/;
const EXPECTED = {
  "test.yml": ["rust-linux", "rust-macos", "rust-windows"],
  "release.yml": ["core", "desktop", "mac"],
};

type Step = { uses?: string; run?: string; if?: string; with?: Record<string, string> };

async function jobs(file: string): Promise<Record<string, { steps: Step[] }>> {
  return Bun.YAML.parse(await Bun.file(join(ROOT, ".github/workflows", file)).text()).jobs;
}

for (const [file, names] of Object.entries(EXPECTED)) {
  test(`${file}: every job that compiles the CLI installs a pinned protoc first`, async () => {
    const all = await jobs(file);
    const compiling = Object.entries(all)
      .filter(([, job]) => job.steps.some((step) => COMPILES.test(step.run ?? "")))
      .map(([name]) => name);
    expect(compiling.sort()).toEqual([...names].sort());
    for (const name of names) {
      const steps = all[name].steps;
      const first = steps.findIndex((step) => COMPILES.test(step.run ?? ""));
      const install = steps.findIndex((step) => ACTION.test(step.uses ?? "") && PINNED.test((step.with?.tool ?? "").trim()));
      expect(install, `${file} ${name}: protoc install step`).toBeGreaterThanOrEqual(0);
      expect(install, `${file} ${name}: protoc before first compile`).toBeLessThan(first);
      expect(steps[install].if, `${file} ${name}: protoc install is unconditional`).toBeUndefined();
      expect(steps[install].with?.fallback, `${file} ${name}: no cargo-binstall fallback`).toBe("none");
      expect(steps[install].with?.checksum, `${file} ${name}: checksum stays on`).toBeUndefined();
    }
  });
}

test("test.yml and release.yml pin the same protoc", async () => {
  const versions = new Set<string>();
  for (const file of Object.keys(EXPECTED)) {
    for (const job of Object.values(await jobs(file))) {
      for (const step of job.steps) {
        const tool = (step.with?.tool ?? "").trim();
        if (ACTION.test(step.uses ?? "") && PINNED.test(tool)) versions.add(tool);
      }
    }
  }
  expect([...versions]).toHaveLength(1);
});

test("test.yml limits the Windows Rust job to one compiler process", async () => {
  const all = (await Bun.YAML.parse(await Bun.file(join(ROOT, ".github/workflows/test.yml")).text())).jobs;
  expect(all["rust-windows"].env?.CARGO_BUILD_JOBS).toBe("1");
});
