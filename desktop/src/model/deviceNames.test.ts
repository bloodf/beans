import { beforeAll, expect, test } from "vitest";
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

test("Device aliases preserve original names, reset blank input, and reject revoked ownership", async () => {
  class DemoStore extends Store { override get isMock() { return true; } }
  const store = new DemoStore();
  store.isConnected = true;
  store.hasIdentity = true;
  store.identityID = "account-a";
  store.devices = [{ id: "peer", name: "Original", model: "PC", os: "linux", osVersion: "", isThisDevice: false, status: "offline", lastSeen: 0, machineKey: "key", plugins: [], version: "" }];
  const owner = store.deviceNamingOwner("peer");
  await store.setDeviceName("peer", "Office", owner);
  expect(store.device("peer")).toMatchObject({ name: "Office", machineName: "Original" });
  await store.setDeviceName("peer", "", owner);
  expect(store.device("peer")).toMatchObject({ name: "Original", machineName: "Original" });
  store.identityID = "account-b";
  expect(owner()).toBe(false);
  await store.setDeviceName("peer", "Wrong account", owner);
  expect(store.device("peer")?.name).toBe("Original");
  const cancelled = store.deviceNamingOwner("peer");
  await store.setDeviceName("peer", "Cancelled", () => false);
  expect(store.device("peer")?.name).toBe("Original");
  store.devices = [];
  expect(cancelled()).toBe(false);
});
