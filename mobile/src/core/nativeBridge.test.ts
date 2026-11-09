import { beforeEach, expect, it, vi } from "vitest";

const native = { start: vi.fn(), previewFile: vi.fn() };
const facts = { name: "Phone", os: "ios", os_version: "27", model: "Phone" };

beforeEach(() => {
  vi.resetModules();
  vi.resetAllMocks();
  vi.doMock("expo-modules-core", () => ({ requireNativeModule: () => native }));
});

it("returns native readiness so callers can await startup", async () => {
  const readiness = Promise.resolve();
  native.start.mockReturnValue(readiness);
  const bridge = await import("../../modules/beans-core");

  expect(bridge.start("/phone/core", facts)).toBe(readiness);
  expect(native.start).toHaveBeenCalledWith("/phone/core", "Phone", "ios", "27", "Phone");
});

it("propagates native startup failure", async () => {
  const bridge = await import("../../modules/beans-core");
  native.start.mockRejectedValue(new Error("Account load failed"));

  await expect(bridge.start("/phone/core", facts)).rejects.toThrow("Account load failed");
});

it("exposes native file preview and propagates its result", async () => {
  const bridge = await import("../../modules/beans-core");
  native.previewFile.mockRejectedValue(new Error("File unavailable"));

  expect(bridge.canPreviewFiles).toBe(true);
  await expect(bridge.previewFile("/phone/output.pdf")).rejects.toThrow("File unavailable");
  expect(native.previewFile).toHaveBeenCalledWith("/phone/output.pdf");
});

it("reports platforms without native preview", async () => {
  vi.doMock("expo-modules-core", () => ({ requireNativeModule: () => ({ start: native.start }) }));
  const bridge = await import("../../modules/beans-core");

  expect(bridge.canPreviewFiles).toBe(false);
  await expect(bridge.previewFile("/phone/output.pdf")).resolves.toBeUndefined();
});
