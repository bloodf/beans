// Whether the chat transcript follows its end as a reply comes in. Pure, so the rule runs under
// bun test.

/** How near the end a scroll the user lets go of has to rest for the transcript to keep following. */
export const FOLLOW_DISTANCE = 24;

/** A scroll the user lets go of `distance` above the end keeps the transcript following. */
export function keepsFollowing(distance: number): boolean {
  return distance <= FOLLOW_DISTANCE;
}

/**
 * New content scrolls the transcript to its end only once the opened chat has settled, with no
 * sent message anchored, for a reader who stayed at the end and has no finger or fling on it.
 */
export function followsEnd(state: {
  settling: boolean;
  anchored: boolean;
  following: boolean;
  userScrolling: boolean;
}): boolean {
  return !state.settling && !state.anchored && state.following && !state.userScrolling;
}
