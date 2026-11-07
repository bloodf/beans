import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { chatContextMenuEntries as getChatContextMenuEntries, openChatContextMenu as getOpenChatContextMenu } from "./sidebar";
import type { select as getSelect, selection as getSelection } from "./root";
import type { store as getStore } from "../model/store";
import type { Bot, Chat } from "../model/models";

let chatContextMenuEntries: typeof getChatContextMenuEntries;
let openChatContextMenu: typeof getOpenChatContextMenu;
let select: typeof getSelect;
let selection: typeof getSelection;
let store: typeof getStore;

const locationDescriptor = Object.getOwnPropertyDescriptor(globalThis, "location");

beforeAll(async () => {
  Object.defineProperty(globalThis, "location", {
    configurable: true,
    value: { search: "?platform=linux", pathname: "/" },
  });
  const sidebar = await import("./sidebar");
  chatContextMenuEntries = sidebar.chatContextMenuEntries;
  openChatContextMenu = sidebar.openChatContextMenu;
  const root = await import("./root");
  select = root.select;
  selection = root.selection;
  const storeModule = await import("../model/store");
  store = storeModule.store;
});

afterAll(() => {
  if (locationDescriptor) Object.defineProperty(globalThis, "location", locationDescriptor);
  else Reflect.deleteProperty(globalThis, "location");
});

const bot: Bot = {
  id: "b2", name: "Bot 2", description: "", symbolName: "sparkles", accent: "blue",
  runnerID: "runner", provider: "deepseek", createdAt: 1,
  capabilities: { shell: false, write: false, plugins: [] },
};
function chat(id: string, kind: Chat["kind"] = "dm"): Chat {
  return { id, kind, isPinned: false, botIDs: [bot.id], messages: [], unreadCount: 0,
    createdAt: 1, hasMore: false, groupDescription: "" };
}

describe("sidebar chat context menu", () => {
  it.each([false, true])("operates on the clicked chat when selection changes during popup: %s", async (changesSelection) => {
    // Seed test selection to chat 1
    const originalSelection = { kind: "chat" as const, id: "c1" };
    select(originalSelection);

    // Mock store.chat
    const c1 = chat("c1");
    const c2 = chat("c2");
    const originalChat = store.chat.bind(store);
    const originalBotsIn = store.botsIn.bind(store);
    const originalTogglePin = store.togglePin.bind(store);

    let pinnedChatID: string | null = null;
    store.chat = (id) => id === "c1" ? c1 : id === "c2" ? c2 : undefined;
    store.botsIn = () => [bot];
    store.togglePin = (id) => { pinnedChatID = id; };

    try {
      // Right-clicking c2 while c1 is selected
      let menuEntries: ReturnType<typeof chatContextMenuEntries> = [];
      await openChatContextMenu("c2", { x: 10, y: 10 }, async (entries) => {
        menuEntries = entries;
        expect(selection.get()).toEqual(originalSelection);
        if (changesSelection) select({ kind: "chat", id: "c3" });
        return "pin";
      });

      // The selection should remain c1
      expect(selection.get()).toEqual(changesSelection ? { kind: "chat", id: "c3" } : originalSelection);
      // But the action (pin) should have been performed on c2
      expect(pinnedChatID).toBe("c2");
      // Menu entries for bot DM should include Pin, Change Look…, separator, Delete Bot
      const labels = menuEntries.map((e) => e.label ?? (e.separator ? "---" : ""));
      expect(labels).toContain("Pin");
      expect(labels).toContain("Change Look…");
      expect(labels).toContain("Delete Bot");
    } finally {
      store.chat = originalChat;
      store.botsIn = originalBotsIn;
      store.togglePin = originalTogglePin;
    }
  });

  it("offers Change Look… for bot chats but not for groups", () => {
    const dmChat = chat("dm");
    const groupChat = chat("grp", "group");

    const originalBotsIn = store.botsIn.bind(store);
    store.botsIn = (chat) => chat.id === "dm" ? [bot] : [];

    try {
      const dmEntries = chatContextMenuEntries(dmChat);
      const groupEntries = chatContextMenuEntries(groupChat);

      expect(dmEntries.some((e) => e.label === "Change Look…")).toBe(true);
      expect(groupEntries.some((e) => e.label === "Change Look…")).toBe(false);
      expect(groupEntries.some((e) => e.label === "Rename…")).toBe(true);
    } finally {
      store.botsIn = originalBotsIn;
    }
  });
});
