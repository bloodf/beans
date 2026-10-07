import { beforeAll, expect, test } from "bun:test";
import type { AppStore } from "./store";

let Store: typeof AppStore;
beforeAll(async () => {
  const location = Object.getOwnPropertyDescriptor(globalThis, "location");
  Object.defineProperty(globalThis, "location", { configurable: true, value: { search: "?platform=linux" } });
  // The browser-only host reads location at import time; static import cannot run first.
  try { Store = (await import("./store")).AppStore; }
  finally {
    if (location) Object.defineProperty(globalThis, "location", location);
    else Reflect.deleteProperty(globalThis, "location");
  }
});

test("the memory bridge rejects arbitrary methods, inherited keys and superseded setup names", async () => {
  const store = new Store();
  for (const method of ["tools.call", "providers.connect_custom", "memory.connections.list.extra", "memory.embeddings.assets.preview", "memory.embeddings.download.apply", "constructor", "toString", "__proto__"]) {
    await expect(store.memoryRequest(method, {})).rejects.toThrow("Unsupported memory request");
  }
});

test("demo mode refuses memory reads and consequential setup without fabricating an adapter", async () => {
  class DemoStore extends Store { override get isMock() { return true; } }
  const store = new DemoStore();
  await expect(store.memoryRequest("memory.connections.list", {})).rejects.toThrow("Memory services are unavailable in demo mode.");
  await expect(store.memoryRequest("memory.embeddings.local.apply", { preview_token: "fixture", preview_digest: "fixture", confirm: true })).rejects.toThrow("Memory services are unavailable in demo mode.");
});
