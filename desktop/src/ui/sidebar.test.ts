import { describe, expect, it } from "vitest";
import { chatContextMenuEntries, openChatContextMenu } from "./sidebar";
import { select, selection } from "./root";
import { store } from "../model/store";

describe("sidebar chat context menu", () => {
  it("leaves the current selection alone and operates on the clicked chat", async () => {
    // Seed test selection to chat 1
    const originalSelection = { kind: "chat" as const, id: "c1" };
    select(originalSelection);

    // Mock store.chat
    const c1 = { id: "c1", kind: "dm", isPinned: false, botIDs: ["b1"], messages: [] };
    const c2 = { id: "c2", kind: "dm", isPinned: false, botIDs: ["b2"], messages: [] };
    const originalChat = store.chat.bind(store);
    const originalBotsIn = store.botsIn.bind(store);
    const originalTogglePin = store.togglePin.bind(store);

    let pinnedChatID: string | null = null;
    store.chat = ((id: string) => (id === "c1" ? c1 : id === "c2" ? c2 : undefined)) as typeof store.chat;
    store.botsIn = ((chat: unknown) => [{ id: "b2", name: "Bot 2" }]) as typeof store.botsIn;
    store.togglePin = ((id: string) => {
      pinnedChatID = id;
    }) as typeof store.togglePin;

    try {
      // Right-clicking c2 while c1 is selected
      let menuEntries: ReturnType<typeof chatContextMenuEntries> = [];
      await openChatContextMenu("c2", { x: 10, y: 10 }, async (entries) => {
        menuEntries = entries;
        return "pin";
      });

      // The selection should remain c1
      expect(selection.get()).toEqual(originalSelection);
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
    const dmChat = { id: "dm", kind: "dm", isPinned: false, botIDs: ["b1"], messages: [] } as any;
    const groupChat = { id: "grp", kind: "group", isPinned: false, botIDs: ["b1", "b2"], messages: [] } as any;

    const originalBotsIn = store.botsIn.bind(store);
    store.botsIn = ((chat: any) => (chat.id === "dm" ? [{ id: "b1", name: "Bot 1" }] : [])) as typeof store.botsIn;

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
