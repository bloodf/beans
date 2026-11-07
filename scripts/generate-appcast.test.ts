import { expect, test } from "vitest";
import { chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { sparkleTool } from "./generate-appcast.ts";

test.skipIf(process.platform === "win32")("Sparkle PATH lookup admits executables and rejects other entries without signing", async () => {
  const directory = await mkdtemp(join(tmpdir(), "beans-sparkle-path-"));
  const savedPath = process.env.PATH;
  const savedBin = process.env.SPARKLE_BIN;
  const name = `probe-${directory.split(/[\\/]/).at(-1)}`;
  const candidate = join(directory, name);
  try {
    delete process.env.SPARKLE_BIN;
    process.env.PATH = directory;
    expect(sparkleTool(name)).toBeNull();
    await mkdir(candidate);
    expect(sparkleTool(name)).toBeNull();
    await rm(candidate, { recursive: true });
    await writeFile(candidate, "#!/bin/sh\nexit 0\n", { mode: 0o600 });
    expect(sparkleTool(name)).toBeNull();
    await chmod(candidate, 0o700);
    expect(sparkleTool(name)).toBe(candidate);
    process.env.PATH = "";
    expect(sparkleTool(name)).toBeNull();
    delete process.env.PATH;
    expect(sparkleTool(name)).toBeNull();
  } finally {
    if (savedPath === undefined) delete process.env.PATH;
    else process.env.PATH = savedPath;
    if (savedBin === undefined) delete process.env.SPARKLE_BIN;
    else process.env.SPARKLE_BIN = savedBin;
    await rm(directory, { recursive: true, force: true });
  }
});
