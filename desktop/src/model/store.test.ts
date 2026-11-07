import { beforeAll, expect, test } from "bun:test";
import type { AppStore } from "./store";

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

/** A store with a transport that answers the given method. */
function withTransport(store: AppStore, request: (method: string, params: Record<string, unknown>) => Promise<unknown>) {
  Object.defineProperty(store, "transport", { value: { request }, configurable: true });
  return store;
}

test("a Compact answers with what the Runner did", async () => {
  const asked: string[] = [];
  const store = withTransport(new Store(), async (method) => { asked.push(method); return null; });
  await store.compactChat("chat-1");
  expect(asked).toEqual(["chats.compact"]);
});

test("a Compact that failed says why, instead of failing silently", async () => {
  const store = withTransport(new Store(), async () => { throw new Error("Runner is offline"); });
  await expect(store.compactChat("chat-1")).rejects.toThrow("Runner is offline");
});

test("the demo compacts without a Runner", async () => {
  class DemoStore extends Store { override get isMock() { return true; } }
  let asked = 0;
  const store = withTransport(new DemoStore(), async () => { asked++; return null; });
  await store.compactChat("chat-1");
  expect(asked).toBe(0);
});
