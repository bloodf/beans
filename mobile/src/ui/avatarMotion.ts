// The numbers an avatar draws on the UI thread, from the shared geometry package. Imports only
// `@beans/blobatar/frame`, whose functions are worklets, so a worklet here can call them. No
// React and no state: a frame is a pure function of (from, to, start, now, amp).

import { avatarFrame, avatarMorphProgress, interpolateAvatarGeometry, type AvatarFrame, type AvatarGeometry } from "@beans/blobatar/frame";

/// The geometry on screen `elapsedMs` into a morph from `from` to `to`; `to` itself once it is
/// over. To retarget an interrupted morph, pass this as the next `from`.
export function displayedGeometry(from: AvatarGeometry, to: AvatarGeometry, elapsedMs: number): AvatarGeometry {
  "worklet";
  const { t, done } = avatarMorphProgress(elapsedMs, to);
  return done ? to : interpolateAvatarGeometry(from, to, t);
}

/// One frame at clock `nowMs`: the morph that began at `startMs`, then the idle layer at
/// amplitude `amp` (0 is the static frame: no breathe, bob, blink, glance, tremor, or seesaw).
export function frameAt(from: AvatarGeometry, to: AvatarGeometry, startMs: number, nowMs: number, amp: number): AvatarFrame {
  "worklet";
  return avatarFrame(displayedGeometry(from, to, nowMs - startMs), nowMs, amp);
}
