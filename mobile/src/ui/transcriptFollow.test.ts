import { expect, test } from "bun:test";
import { followsEnd, keepsFollowing } from "./transcriptFollow";

test("a reader who scrolled up while a reply comes in stays where they are", () => {
  const stayed = { settling: false, anchored: false, following: true, userScrolling: false };
  expect(followsEnd(stayed)).toBe(true);
  expect(followsEnd({ ...stayed, following: false })).toBe(false);
  expect(followsEnd({ ...stayed, userScrolling: true })).toBe(false);
  expect(followsEnd({ ...stayed, settling: true })).toBe(false);
  expect(followsEnd({ ...stayed, anchored: true })).toBe(false);
  expect(keepsFollowing(24)).toBe(true);
  expect(keepsFollowing(25)).toBe(false);
});
