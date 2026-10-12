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

test("archive inspection rejects encrypted, overlapping and miscounted entries before any tool spawn", () => {
  const encrypted = archiveFixture(), central = encrypted.indexOf(Buffer.from([0x50, 0x4b, 1, 2]));
  encrypted.writeUInt16LE(1, central + 8); // general purpose bit 0: encrypted
  expect(() => inspectReleaseArchive(encrypted)).toThrow("Unsafe binary archive");
  expect(() => inspectReleaseArchive(encrypted)).toThrow("decompressed size");

  const miscounted = archiveFixture(), end = miscounted.lastIndexOf(Buffer.from([0x50, 0x4b, 5, 6]));
  miscounted.writeUInt16LE(2, end + 10); // EOCD claims two entries, one present
  expect(() => inspectReleaseArchive(miscounted)).toThrow("Unsafe binary archive");

  // Span overlap is deliberately NOT asserted here: a fixture with two
  // individually-valid entries whose declared spans overlap is rejected earlier by
  // the per-entry and EOCD guards, so it never reaches the span check and would not
  // prove it. Unproven rather than falsely claimed.
});
