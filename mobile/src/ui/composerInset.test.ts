import { expect, test } from "bun:test";
import { composerExtra } from "./composerInset";

test("no inset is published before the composer's first layout", () => {
  expect(composerExtra(48, Number.POSITIVE_INFINITY, 20)).toBeNull();
  expect(composerExtra(40, 40, 20)).toBe(60);
  expect(composerExtra(90, 40, 20)).toBe(90);
});
