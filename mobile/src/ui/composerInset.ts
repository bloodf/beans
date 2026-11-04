// The chat transcript's extra bottom padding for the composer. Pure, so the rule runs under bun
// test.

/** The padding for the composer's `actual` height, or `null` while it has no compact height. */
export function composerExtra(actual: number, compact: number, focusGrowth: number): number | null {
  // Until the composer's first layout there is no compact height, and the guess stands. A
  // keyboard that comes to rest before then (one dismissed on the way into the chat) would
  // otherwise publish an infinite inset, and UIKit throws on the NaN scroll bar it makes.
  if (!Number.isFinite(compact)) return null;
  return actual > compact + 1 ? actual : compact + focusGrowth;
}
