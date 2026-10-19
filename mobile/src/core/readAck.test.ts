import { expect, mock, test } from "bun:test";
import type { Chat } from "./model";

// Replace only OS I/O. The store and bundled JSON request/error decoder are real.
const pending: { resolve: (raw: string) => void; reject: (error: Error) => void }[] = [];
mock.module("expo-modules-core", () => ({
  requireNativeModule: () => ({
    request: (_method: string, _params: string) => new Promise<string>((resolve, reject) => pending.push({ resolve, reject })),
  }),
}));
mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {} }));
const { markRead, resetStore, useStore } = await import("./store");
const chat = (unread_count: number): Chat => ({ id: "chat", kind: "dm", bot_ids: [], is_pinned: false, created_at: 0, messages: [], unread_count });
const unread = () => useStore.getState().chats[0].unread_count;
const settle = async () => { await new Promise((resolve) => setTimeout(resolve, 0)); };

test("read acknowledgement keeps failures and revoked or newer unread state honest", async () => {
  resetStore();
  useStore.setState({ paired: true, identityId: "synthetic-account", deviceId: "phone", appActive: true, chats: [chat(3)] });
  markRead("chat");
  expect(unread()).toBe(3); // Not persisted yet.
  await settle();
  pending.shift()!.resolve(JSON.stringify({ error: { message: "persistence denied" } }));
  await settle();
  expect(unread()).toBe(3);

  markRead("chat");
  await settle();
  pending.shift()!.reject(new Error("native request failed"));
  await settle();
  expect(unread()).toBe(3);

  markRead("chat");
  await settle();
  pending.shift()!.resolve('{"result":null}');
  await settle();
  expect(unread()).toBe(0);

  for (const change of [
    () => { useStore.setState({ appActive: false }); useStore.setState({ appActive: true }); },
    () => { useStore.setState({ identityId: "other-account" }); useStore.setState({ identityId: "synthetic-account" }); },
    () => { useStore.setState({ deviceId: "other-phone" }); useStore.setState({ deviceId: "phone" }); },
    () => { useStore.setState({ relayUrl: "synthetic-relay" }); useStore.setState({ relayUrl: null }); },
    () => { resetStore(); useStore.setState({ paired: true, identityId: "synthetic-account", deviceId: "phone", chats: [chat(3)] }); },
    () => { useStore.setState({ chats: [chat(4)] }); },
  ]) {
    useStore.setState({ chats: [chat(3)] });
    markRead("chat");
    await settle();
    const old = pending.shift()!;
    change();
    const expected = unread();
    old.resolve('{"result":null}');
    await settle();
    expect(unread()).toBe(expected);
    markRead("chat");
    await settle();
    pending.shift()!.resolve('{"result":null}');
    await settle();
    expect(unread()).toBe(0);
  }

  useStore.setState({ chats: [chat(5)] });
  markRead("chat");
  const revoked = pending.shift()!;
  useStore.setState({ appActive: false });
  useStore.setState({ appActive: true });
  markRead("chat");
  const current = pending.shift()!;
  revoked.resolve('{"result":null}');
  await settle();
  expect(unread()).toBe(5);
  current.resolve('{"result":null}');
  await settle();
  expect(unread()).toBe(0);

  useStore.setState({ chats: [chat(2)], appActive: false });
  markRead("chat");
  await settle();
  expect(unread()).toBe(2);
  expect(pending).toEqual([]);
  useStore.setState({ appActive: true, paired: false });
  markRead("chat");
  await settle();
  expect(unread()).toBe(2);
  expect(pending).toEqual([]);
});
