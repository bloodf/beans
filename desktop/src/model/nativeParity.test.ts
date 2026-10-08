import { expect, test } from "bun:test";
import { botAvatarGeometry, avatarFrame } from "../../../packages/beans-blobatar/index";
import { MemoryPreferencesDraft, readMemoryConnections, MemoryServiceViewState } from "./memoryService";

test("native geometry input keeps stable identity and sparse state inheritance", () => {
  const look = { version: 1 as const, base: { palette: { head: "#ABCDEF" } }, states: { working: { palette: { eye: "#123456" } } } };
  const geometry = botAvatarGeometry("bot-native", look, "working");
  const frame = avatarFrame(geometry, 0, 0);
  expect(frame.head).toBe("#ABCDEF");
  expect(frame.eye).toBe("#123456");
  expect(botAvatarGeometry("bot-native", look, "idle").palette.head).toBe("#ABCDEF");
});

test("native memory inputs fail closed and masked replies discard private fields", () => {
  const draft = new MemoryPreferencesDraft("bot-native");
  draft.connectionID = "connection"; draft.captureConversation = true;
  expect(() => draft.request()).toThrow("plaintext_consent_required");
  draft.approveRemotePlaintext();
  const prefs = { ...draft.request().params, consent_revision: { counter: 1, device_id: "d" }, deletion_epoch: 3 };
  const state = new MemoryServiceViewState("bot-native", prefs);
  state.draft.maxCaptureDeliveriesPerTurn = 2;
  state.beginDeletion(); state.recordDeletionEpoch(4);
  expect(() => state.refreshPreferences(prefs)).toThrow("stale_preferences");
  expect(state.deletionLocked).toBe(true);
  const masked = readMemoryConnections({ schema_version: 1, connections: [{ id: "c", revision: { counter: 1, device_id: "d" }, backend: "hindsight", name: "Memory", has_secret: true, embedding_profile: null, availability: "supported", reason: null, secret: "private", endpoint: "private" }], embeddings: [], bots: [] });
  expect(JSON.stringify(masked)).not.toContain("private");
});
