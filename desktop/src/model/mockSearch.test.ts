// Issue #586: the demo (`?mock=1`) shows Settings › Search and the palette's Related and
// Pictures sections with content, and never installs a model.

import { expect, test } from "vitest";
import { chats } from "./mock";
import { mockLocalModels, mockModelPreview, mockRelated, mockSearchChoice, mockSearchIndexStatus } from "./mockSearch";
import { downloadOrder } from "./localModels";

test("the demo Runner searches with a text and an image model, both ready", () => {
  const status = mockSearchIndexStatus(mockSearchChoice());
  expect(status.text).toMatchObject({ state: "ready", choice: { kind: "local", model_id: "multilingual-e5-small" } });
  expect(status.image).toMatchObject({ state: "ready", choice: "clip-vit-b32" });
  expect(mockSearchIndexStatus({ text: null, image: null }).text.state).toBe("off");
});

test("the catalog has a suggestion, installed models, and one that does not fit", () => {
  const listing = mockLocalModels(mockSearchChoice());
  expect(listing.recommended?.text).toBe("multilingual-e5-small");
  expect(listing.models.find((model) => model.id === listing.recommended?.image)?.installed).toBe(false);
  expect(listing.models.some((model) => model.fit?.fits === false)).toBe(true);
  expect(listing.models.filter((model) => model.installed).map((model) => model.id)).toEqual(["multilingual-e5-small", "clip-vit-b32"]);
  expect(listing.disk_bytes).toBe(135_000_000 + 605_000_000);
  expect(downloadOrder(listing.models, listing.recommended?.image)[0]?.id).toBe("siglip-base");
  const preview = mockModelPreview(listing, "siglip-base");
  expect(preview).toMatchObject({ name: "SigLIP Base", hub: "Hugging Face", total_bytes: 813_000_000 });
});

test("related messages and pictures come from the demo chats by meaning", () => {
  const demo = chats();
  const found = mockRelated("what is left before we ship?", demo, mockSearchChoice());
  expect(found.state).toBe("ready");
  expect(found.related.length).toBeGreaterThan(0);
  for (const hit of found.related) {
    const chat = demo.find((each) => each.id === hit.chat_id);
    expect(chat?.messages.some((message) => message.id === hit.message_id)).toBe(true);
  }
  expect(found.related.some((hit) => /launch|announcement/i.test(hit.snippet))).toBe(true);
  expect(found.images?.map((hit) => hit.name)).toEqual(["launch-banner.png"]);
  expect(mockRelated("pairing on my phone", demo, mockSearchChoice()).images?.map((hit) => hit.name)).toEqual(["pairing-screen.png"]);
  expect(mockRelated("zebra crossing", demo, mockSearchChoice()).related).toEqual([]);
  expect(mockRelated("ship", demo, { text: null, image: null })).toMatchObject({ state: "off", related: [], images: [] });
});
