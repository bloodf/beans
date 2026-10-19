import { expect, test } from "bun:test";
import { UpdateController } from "./updateController";

const offer = { id: "snapshot", version: "1.10.0", notes: "untrusted notes" };
test("checks and background never install; consent expires on dismissal", async () => {
  let installs = 0;
  const controller = new UpdateController({ check: async () => offer, install: async () => { installs++; }, cancel: () => {} });
  await controller.check();
  expect(installs).toBe(0);
  controller.dismiss();
  await expect(controller.install("snapshot", false)).rejects.toThrow("Consent expired");
  await controller.check();
  await expect(controller.install("snapshot", true)).rejects.toThrow("Save or send drafts");
  expect(installs).toBe(0);
  await controller.install("snapshot", false);
  expect(installs).toBe(1);
  await expect(controller.install("snapshot", false)).rejects.toThrow("Consent expired");
});
test("late check cannot resurrect background consent and concurrent checks collapse", async () => {
  let resolve!: (value: typeof offer) => void;
  let checks = 0;
  const controller = new UpdateController({ check: () => { checks++; return new Promise(r => { resolve = r; }); }, install: async () => {}, cancel: () => {} });
  const pending = controller.check();
  await controller.check();
  expect(checks).toBe(1);
  controller.dismiss();
  resolve(offer);
  await pending;
  expect(controller.offer).toBeNull();
});
