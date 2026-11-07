import { createSignal, flush } from "solid-js";
import { render } from "@solidjs/web";
import { setLanguage } from "../l10n";
import { ActionRow } from "./sections";

export async function checkCompactActionRow(): Promise<Record<string, string>> {
  const assert = (value: unknown, message: string) => {
    if (!value) throw new Error(message);
  };
  setLanguage("en");
  const root = document.createElement("div");
  document.body.append(root);
  const [phase, setPhase] = createSignal<"idle" | "running" | "done">("idle");
  let requests = 0;
  const dispose = render(() => (
    <ActionRow label="Context" value="128k" busy={phase() === "running"}
      actionTitle={phase() === "running" ? "Compacting…" : phase() === "done" ? "Compacted" : "Compact"}
      actionCopied={phase() === "done"}
      onAction={() => { requests++; setPhase("running"); }} />
  ), root);
  const settle = async () => { await Promise.resolve(); flush(); };
  try {
    await settle();
    const button = root.querySelector<HTMLButtonElement>("button");
    assert(button?.textContent?.trim() === "Compact", "Idle Compact action missing");
    button?.click();
    await settle();
    const progress = root.querySelector<HTMLElement>('[role="progressbar"]');
    assert(progress?.getAttribute("aria-label") === "Compacting…", "Busy Compact progress has no accessible name");
    assert(root.querySelector("button") === null, "Busy Compact still exposes an action");
    root.click();
    assert(requests === 1, "Busy row dispatched a duplicate action");
    setPhase("done");
    await settle();
    assert(root.querySelector('[role="progressbar"]') === null, "Progress remains after completion");
    assert(root.querySelector("button.copied")?.textContent?.trim() === "Compacted", "Success check/title did not react");
    setPhase("idle");
    await settle();
    assert(root.querySelector("button")?.textContent?.trim() === "Compact", "Idle action did not return");
    return { compact: "named progress and reactive running/done/idle action verified" };
  } finally {
    dispose();
    root.remove();
  }
}
