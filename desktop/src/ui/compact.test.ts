import { expect, test } from "bun:test";
import { Compacts, compactShown, type CompactScheduler } from "./compact";

/** A clock the test moves by hand, so a two-second moment costs nothing. */
function fixture() {
  let now = 0;
  let next = 1;
  const pending = new Map<number, { at: number; handler: () => void }>();
  const scheduler: CompactScheduler = {
    setTimeout(handler, ms) {
      const id = next++;
      pending.set(id, { at: now + ms, handler });
      return id;
    },
    clearTimeout(id) {
      pending.delete(id);
    },
  };
  const advance = (ms: number) => {
    now += ms;
    for (const [id, timer] of [...pending]) {
      if (timer.at > now) continue;
      pending.delete(id);
      timer.handler();
    }
  };
  return { compacts: new Compacts(scheduler, () => {}), advance, pending };
}

test("a Compact under way is running, and a second click sends nothing", () => {
  const { compacts } = fixture();
  expect(compacts.phase("chat-1")).toBe("idle");
  expect(compacts.start("chat-1")).toBe(true);
  expect(compacts.phase("chat-1")).toBe("running");
  expect(compacts.start("chat-1")).toBe(false);
  expect(compacts.phase("chat-1")).toBe("running");
});

test("a Compact that worked is checked off for two seconds, then the row is idle again", () => {
  const { compacts, advance } = fixture();
  compacts.start("chat-1");
  compacts.succeed("chat-1");
  expect(compacts.phase("chat-1")).toBe("done");
  advance(compactShown - 1);
  expect(compacts.phase("chat-1")).toBe("done");
  advance(1);
  expect(compacts.phase("chat-1")).toBe("idle");
});

test("a Compact that failed says why in the toast, and the row goes back to Compact", () => {
  const { compacts, advance } = fixture();
  compacts.start("chat-1");
  compacts.fail("chat-1", "Runner is offline");
  expect(compacts.phase("chat-1")).toBe("idle");
  expect(compacts.toast()).toBe("Runner is offline");
  advance(compactShown - 1);
  expect(compacts.toast()).toBe("Runner is offline");
  advance(1);
  expect(compacts.toast()).toBe("");
});

test("a late timer does not undo a newer Compact", () => {
  const { compacts, advance } = fixture();
  compacts.start("chat-1");
  compacts.succeed("chat-1");
  compacts.start("chat-1");
  advance(compactShown);
  expect(compacts.phase("chat-1")).toBe("running");
  compacts.succeed("chat-1");
  expect(compacts.phase("chat-1")).toBe("done");
});

test("a reply for a Compact that is not under way changes nothing", () => {
  const { compacts } = fixture();
  compacts.start("chat-1");
  compacts.fail("chat-1", "Nothing to compact yet");
  compacts.succeed("chat-1");
  expect(compacts.phase("chat-1")).toBe("idle");
  expect(compacts.toast()).toBe("Nothing to compact yet");
});

test("one chat's Compact leaves another's alone", () => {
  const { compacts } = fixture();
  compacts.start("chat-1");
  compacts.start("chat-2");
  compacts.fail("chat-1", "Runner is offline");
  expect(compacts.phase("chat-1")).toBe("idle");
  expect(compacts.phase("chat-2")).toBe("running");
});
