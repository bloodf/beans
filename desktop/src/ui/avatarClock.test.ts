import { expect, test } from "bun:test";
import { AvatarClock } from "./avatarClock";

function fixture() {
  let next = 1;
  const scheduled = new Map<number, FrameRequestCallback>();
  const clock = new AvatarClock({ request: (callback) => { const id = next++; scheduled.set(id, callback); return id; }, cancel: (id) => { scheduled.delete(id); } });
  const tick = (time: number) => {
    const pending = [...scheduled];
    scheduled.clear();
    for (const [, callback] of pending) callback(time);
  };
  return { clock, scheduled, tick };
}

test("visible avatars share one frame request and the final offscreen/unmounted unsubscribe cancels it", () => {
  const { clock, scheduled, tick } = fixture();
  const first: number[] = [], second: number[] = [];
  expect(scheduled.size).toBe(0);
  const hideFirst = clock.subscribe((time) => first.push(time));
  const hideSecond = clock.subscribe((time) => second.push(time));
  expect(scheduled.size).toBe(1);
  tick(16);
  expect(first).toEqual([16]); expect(second).toEqual([16]);
  expect(scheduled.size).toBe(1);
  hideFirst(); tick(32);
  expect(first).toEqual([16]); expect(second).toEqual([16, 32]);
  hideSecond();
  expect(scheduled.size).toBe(0);
  tick(48);
  expect(second).toEqual([16, 32]);
});

test("a frame listener may stop itself without scheduling a ghost callback, and resuming starts one clock", () => {
  const { clock, scheduled, tick } = fixture();
  let stop = () => {};
  let calls = 0;
  stop = clock.subscribe(() => { calls++; stop(); });
  tick(16);
  expect(calls).toBe(1);
  expect(scheduled.size).toBe(0);
  const pause = clock.subscribe(() => { calls++; });
  expect(scheduled.size).toBe(1);
  tick(32); pause();
  expect(calls).toBe(2);
  expect(scheduled.size).toBe(0);
});
