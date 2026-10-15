import { beforeAll, expect, test } from "bun:test";
import type { AppStore, SkillLibraryCompare } from "./store";
import { toDevice } from "./wire";

let Store: typeof AppStore;
beforeAll(async () => {
  const location = Object.getOwnPropertyDescriptor(globalThis, "location");
  Object.defineProperty(globalThis, "location", { configurable: true, value: { search: "?platform=linux" } });
  try { Store = (await import("./store")).AppStore; }
  finally {
    if (location) Object.defineProperty(globalThis, "location", location);
    else Reflect.deleteProperty(globalThis, "location");
  }
});

const runnerID = "synthetic-runner", managedID = "b".repeat(32), root = "/synthetic-source";
const reply: SkillLibraryCompare = { version: 1, runner_id: runnerID, managed_id: managedID, added: [], removed: [], changed: [] };
const labels = (prefix: string, count: number) => Array.from({ length: count }, (_, i) => `${prefix}${String(i).padStart(3, "0")}`);

function consumer(value: unknown) {
  class TestStore extends Store { override get isMock() { return false; } }
  const store = new TestStore();
  store.devices = [toDevice({ id: runnerID, name: "Synthetic Linux", os: "linux", model: "Fixture", os_version: "fixture", is_this_device: false, status: "online", last_seen: 1, machine_key: "synthetic-key", plugins: [] })];
  store.identityID = "synthetic-account";
  store.hasIdentity = true;
  store.isConnected = true;
  const calls: { method: string; params: Record<string, unknown> }[] = [];
  Object.defineProperty(store, "transport", { value: { request: async (method: string, params: Record<string, unknown>) => {
    calls.push({ method, params });
    return value;
  } } });
  expect(calls).toHaveLength(0);
  return { store, calls };
}

function readOnly(calls: ReturnType<typeof consumer>["calls"]) {
  expect(calls).toEqual([{ method: "skills.library.compare", params: { version: 1, runner_id: runnerID, root, managed_id: managedID } }]);
}

test("Compare accepts 256 combined labels but refuses 257 without retry or mutation", async () => {
  for (const counts of [[86, 85, 85], [256, 0, 0], [0, 256, 0], [0, 0, 256]]) {
    const value = { ...reply, added: labels("a", counts[0]!), removed: labels("r", counts[1]!), changed: labels("c", counts[2]!) };
    const { store, calls } = consumer(value);
    expect(await store.compareSkillLibrary(runnerID, root, managedID)).toEqual(value);
    readOnly(calls);
  }
  for (const counts of [[86, 86, 85], [257, 0, 0], [0, 257, 0], [0, 0, 257]]) {
    const { store, calls } = consumer({ ...reply, added: labels("a", counts[0]!), removed: labels("r", counts[1]!), changed: labels("c", counts[2]!) });
    await expect(store.compareSkillLibrary(runnerID, root, managedID)).rejects.toThrow();
    readOnly(calls);
  }
});

test("Compare validates Rust UTF-8 ordering, not JavaScript UTF-16 ordering", async () => {
  const rustOrder = ["prefix/", "prefix/\u{10000}"];
  expect([...rustOrder].sort()).toEqual([...rustOrder].reverse());
  for (const field of ["added", "removed", "changed"] as const) {
    const value = { ...reply, [field]: rustOrder };
    const { store, calls } = consumer(value);
    expect(await store.compareSkillLibrary(runnerID, root, managedID)).toEqual(value);
    readOnly(calls);
    const reversed = consumer({ ...reply, [field]: [...rustOrder].reverse() });
    await expect(reversed.store.compareSkillLibrary(runnerID, root, managedID)).rejects.toThrow();
    readOnly(reversed.calls);
  }
});

test("Compare refuses malformed shapes, labels, duplicates and overlapping lists without automatic effects", async () => {
  const malformed = [
    null, { ...reply, body: "PRIVATE" }, { ...reply, version: 2 },
    { ...reply, runner_id: "other-runner" }, { ...reply, managed_id: "a".repeat(32) },
    { ...reply, managed_id: "../copy" }, { ...reply, added: null }, { ...reply, removed: "path" },
    { version: 1, runner_id: runnerID, managed_id: managedID, added: [], removed: [] },
    ...["", "/absolute", "../escape", "a/../b", "a/./b", "a//b", "a/", "a\\b", "a:b", "a\n", "a\u007f", "a\u0085", "a/b/c/d/e/f", "é".repeat(2049), 1].map((label) => ({ ...reply, added: [label] })),
    { ...reply, added: ["z", "a"] }, { ...reply, added: ["aa", "a"] }, { ...reply, added: ["a", "a"] },
    { ...reply, added: ["a"], removed: ["a"] }, { ...reply, added: ["a"], changed: ["a"] },
    { ...reply, removed: ["a"], changed: ["a"] },
  ];
  for (const value of malformed) {
    const { store, calls } = consumer(value);
    await expect(store.compareSkillLibrary(runnerID, root, managedID)).rejects.toThrow();
    readOnly(calls);
  }
  const boundary = { ...reply, added: ["a/b/c/d/e", "é".repeat(2048)] };
  const { store, calls } = consumer(boundary);
  expect(await store.compareSkillLibrary(runnerID, root, managedID)).toEqual(boundary);
  readOnly(calls);
});
