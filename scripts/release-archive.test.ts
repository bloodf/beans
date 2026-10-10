import { expect, test } from "bun:test";
import { inspectReleaseArchive } from "./release-build-inputs.ts";
import { archiveFixture } from "./release-archive-fixture.ts";

test("archive inspection accepts bounded format and rejects escape, symlink and decompression claims", () => {
  expect(inspectReleaseArchive(archiveFixture())).toEqual(["fixture.txt"]);
  for (const name of ["../escape", "/absolute", "C:/escape", "dir/../escape", "dir\\escape", "wild*.plist"]) {
    expect(() => inspectReleaseArchive(archiveFixture(name))).toThrow("Unsafe binary archive");
  }
  expect(() => inspectReleaseArchive(archiveFixture("link", "../../escape", 0xa1ff))).toThrow("Unsafe binary archive");
  const oversized = archiveFixture(), central = oversized.indexOf(Buffer.from([0x50, 0x4b, 1, 2]));
  oversized.writeUInt32LE(128 * 1024 * 1024 + 1, central + 24);
  expect(() => inspectReleaseArchive(oversized)).toThrow("decompressed size");
  const mismatch = archiveFixture();
  mismatch.writeUInt32LE(1, central + 24);
  expect(() => inspectReleaseArchive(mismatch)).toThrow("decompressed size");
});
