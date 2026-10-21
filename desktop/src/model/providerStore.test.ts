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

test("discovered aliases keep their verbatim identity and order when saved", async () => {
  const store = new Store();
  const ids = [" exact-alias ", "exact-alias", "\tcombo/ALIAS\n"];
  let saved: Record<string, unknown> | undefined;
  Object.defineProperty(store, "transport", { value: {
    request: async (method: string, params: Record<string, unknown>) => {
      if (method === "providers.list_models") return { listed: true, models: ids.map((id) => ({ id })) };
      if (method === "providers.connect_custom") {
        saved = params;
        return { kind: "custom:aliases" };
      }
      throw new Error(`Unexpected method: ${method}`);
    },
  } });
  const options = { kind: "custom:aliases" as const, name: "Historical aliases", api: "chat-completions" as const, baseURL: "http://fixture.invalid/v1", apiKey: "" };
  const listed = await store.listCustomModels(options);
  await store.saveCustomProvider({ ...options, models: [...listed!.map((model) => model.id), "", " \t\n "] });
  expect(saved?.models).toEqual(ids);
});

test("demo saves reject blank IDs without rewriting nonblank aliases", async () => {
  class DemoStore extends Store { override get isMock() { return true; } }
  const store = new DemoStore();
  await store.saveCustomProvider({ kind: "custom:aliases", name: "Aliases", api: "messages", baseURL: "http://fixture.invalid", apiKey: "", models: ["", " \n\t ", " exact-alias ", "exact-alias"] });
  expect(store.providers[0]!.models!.map((model) => model.id)).toEqual([" exact-alias ", "exact-alias"]);
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
