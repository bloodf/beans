// Whether a roster event carries the custom provider the sheet was refreshing, or only an unrelated
// account change. Model ids alone never settle it: the same ids with new metadata is a real update.

import { expect, test } from "bun:test";
import { refreshSettled } from "./models";
import type { ProviderCredential } from "./models";

const saved: ProviderCredential = {
  kind: "custom:lab",
  isConnected: true,
  detail: "",
  name: "Lab",
  api: "chat-completions",
  baseURL: "http://localhost:11434/v1",
  models: [{ id: "qwen3", levels: ["low"] }],
};

test("an unrelated roster event leaves the refresh unsettled", () => {
  expect(refreshSettled(saved, { ...saved })).toBe(false);
  expect(refreshSettled(saved, undefined)).toBe(false);
});

test("new model ids from the refreshed catalog settle it", () => {
  const after: ProviderCredential = { ...saved, models: [...saved.models!, { id: "gemma3", levels: ["low"] }] };
  expect(refreshSettled(saved, after)).toBe(true);
});

test("the same ids with changed metadata settle it too", () => {
  const after = { ...saved, models: [{ id: "qwen3", levels: ["low", "high"], contextWindow: 32768 }] };
  expect(refreshSettled(saved, after)).toBe(true);
});

test("another provider's catalog settles nothing", () => {
  const other: ProviderCredential = { kind: "custom:other", isConnected: true, detail: "", models: [{ id: "new", levels: [] }] };
  expect(refreshSettled(saved, other)).toBe(false);
});

test("a dropped or disconnected provider settles nothing", () => {
  expect(refreshSettled(saved, { ...saved, models: [] })).toBe(false);
  expect(refreshSettled(saved, { ...saved, isConnected: false })).toBe(false);
});

test("a repointed provider on another endpoint settles nothing", () => {
  expect(refreshSettled(saved, { ...saved, baseURL: "http://127.0.0.1:1234/v1" })).toBe(false);
  expect(refreshSettled(saved, { ...saved, api: "responses", models: [{ id: "other", levels: [] }] })).toBe(false);
});
