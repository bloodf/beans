import { beforeEach, expect, test, vi } from "vitest";

vi.mock("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {} }));
vi.mock("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
const { customRequestURL } = await import("./model");
const { resetStore, setRelayStatus, useStore } = await import("./store");

beforeEach(() => {
  resetStore();
  useStore.setState({ pendingTemplateLink: null });
});

test("custom provider previews retain Messages roots and decision endpoints", () => {
  expect(customRequestURL("messages", "https://api.anthropic.com/v1/models")).toBe("https://api.anthropic.com/v1/messages");
  expect(customRequestURL("messages", "https://vendor.example/anthropic")).toBe("https://vendor.example/anthropic/v1/messages");
  expect(customRequestURL("decisions", "https://vendor.example/v1/systemone")).toBe("https://vendor.example/v1/systemone");
});

test("reset preserves an unopened template link and clears account state", () => {
  useStore.setState({ pendingTemplateLink: "https://example.test/shared", paired: true, identityId: "previous", paused: true });
  resetStore();
  expect(useStore.getState().pendingTemplateLink).toBe("https://example.test/shared");
  expect(useStore.getState().paired).toBe(false);
  expect(useStore.getState().identityId).toBeNull();
  expect(useStore.getState().paused).toBe(false);
});

test("changing relay discards provider catalogs belonging to the prior source", () => {
  useStore.setState({ relayUrl: "https://old.test", models: [{ provider: "custom:old", id: "old", name: "Old", levels: [] }], providers: [] });
  setRelayStatus({ url: "https://new.test", connected: false });
  expect(useStore.getState().models).toEqual([]);
  expect(useStore.getState().relayUrl).toBe("https://new.test");
});
