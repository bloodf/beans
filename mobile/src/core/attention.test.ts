import { expect, mock, test } from "bun:test";
import type { AttentionBrief, AttentionItem } from "./attention";
import type { ReviewItem } from "./model";

mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));

const { emptyAttention } = await import("./attention");
const { reviewIsOpen } = await import("./model");
const { useStore } = await import("./store");

const revision = { counter: 1, device_id: "phone" };

const brief = (extra: Partial<AttentionBrief> = {}): AttentionBrief => ({
  coordinator_bot_id: "chef",
  chat_id: "dm-chef",
  decisions: ["Ship TLS"],
  changes: [],
  next_action: "Approve the notes",
  revision,
  updated_at: 1,
  ...extra,
});

const item = (extra: Partial<AttentionItem> = {}): AttentionItem => ({
  id: "attn-1",
  category: "review",
  title: "Release notes",
  summary: "Needs a look",
  next_action: "Approve or leave notes",
  coordinator_bot_id: "chef",
  sources: [{ chat_id: "chat-quill", review_id: "review-1" }],
  urgent: false,
  revision,
  ...extra,
});

const review = (extra: Partial<ReviewItem> = {}): ReviewItem => ({
  id: "review-1",
  runner_id: "mac",
  bot_id: "bot",
  origin: { chat_id: "chat-quill" },
  target: { account: "Workbench", resource: "~/relay" },
  rationale: "Pushes a tag",
  payload: { kind: "shell", arguments: { command: "git push --tags" } },
  version: 1,
  revision: 1,
  preconditions: { workdir: "~/relay", files: [] },
  state: "pending",
  created_at: 1,
  ...extra,
});

test("emptyAttention starts with briefs and urgent items on", () => {
  expect(emptyAttention()).toEqual({
    items: [],
    briefs: [],
    preferences: { summaries: true, urgent_direct: true, default_coordinator_bot_id: null },
  });
});

test("dismissing a brief removes it and leaves linked items until they resolve", () => {
  useStore.setState({
    attention: {
      items: [item()],
      briefs: [brief()],
      preferences: emptyAttention().preferences,
    },
    reviews: [],
    chats: [{ id: "chat-quill", kind: "dm", bot_ids: ["bot"], is_pinned: false, created_at: 1, messages: [], unread_count: 0 } as any],
  });
  // The phone asks the core to dismiss; the core answers with attention.changed without the brief.
  useStore.setState({
    attention: {
      items: [item()],
      briefs: [],
      preferences: emptyAttention().preferences,
    },
  });
  expect(useStore.getState().attention.briefs).toEqual([]);
  expect(useStore.getState().attention.items).toHaveLength(1);
  useStore.setState({
    attention: {
      items: [],
      briefs: [],
      preferences: emptyAttention().preferences,
    },
  });
  expect(useStore.getState().attention.items).toEqual([]);
});

test("an Attention row that waits on an open review opens that review on iOS and Android", () => {
  for (const os of ["ios", "android"] as const) {
    const chats = [{ id: "chat-quill" }];
    const reviews = [review()];
    const row = item();
    const found = row.sources
      .map((source) => source.review_id && reviews.find((each) => each.id === source.review_id))
      .find((each): each is ReviewItem => !!each && reviewIsOpen(each) && !each.is_message && chats.some((chat) => chat.id === each.origin.chat_id));
    expect(found?.id, os).toBe("review-1");
    expect(`/chat-info/review/${found!.id}`, os).toBe("/chat-info/review/review-1");
    expect(`/chat/${found!.origin.chat_id}`, os).toBe("/chat/chat-quill");
  }
  expect(reviewIsOpen(review({ state: "succeeded" }))).toBe(false);
  expect(
    item({ sources: [{ chat_id: "chat-quill", review_id: "review-1" }] }).sources
      .map((source) => source.review_id && [review({ state: "succeeded" })].find((each) => each.id === source.review_id))
      .find((each) => !!each && reviewIsOpen(each as ReviewItem)),
  ).toBeUndefined();
});
