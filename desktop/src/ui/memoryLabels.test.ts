import { expect, test } from "bun:test";
import { chosenLanguage, setLanguage } from "../l10n";
import type { MemoryHealth, MemoryOperationState } from "../model/memoryService";
import { memoryHealthLabel, memoryOperationLabel } from "./memoryLabels";

test("memory delivery labels distinguish unconfirmed storage in both languages", () => {
  const previousLanguage = chosenLanguage();
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
  // Language selection updates this DOM attribute; no UI rendering is claimed.
  if (!previousDocument) Object.defineProperty(globalThis, "document", { configurable: true, value: { documentElement: { lang: "" } } });
  try {
    const states: MemoryOperationState[] = ["queued", "submitted", "processing", "completed", "failed", "delivery_unknown"];
    for (const language of ["en", "zh-Hans"] as const) {
      setLanguage(language);
      const labels = states.map(memoryOperationLabel);
      expect(new Set(labels).size).toBe(states.length);
      expect(states.filter(state => memoryOperationLabel(state) === (language === "en" ? "Stored" : "已存储"))).toEqual(["completed"]);
      for (const state of states.filter(state => state !== "completed")) {
        expect(memoryOperationLabel(state)).toContain(language === "en" ? "not" : "未");
      }
      expect(memoryOperationLabel("delivery_unknown")).toBe(language === "en" ? "Delivery unknown, not confirmed stored" : "投递状态未知，未确认存储");
      const statuses: MemoryHealth["status"][] = ["ready", "degraded", "setup_required"];
      expect(statuses.map(memoryHealthLabel))
        .toEqual(language === "en" ? ["Ready", "Degraded", "Setup required"] : ["就绪", "降级", "需要设置"]);
    }
  } finally {
    setLanguage(previousLanguage);
    if (!previousDocument) Reflect.deleteProperty(globalThis, "document");
  }
});
