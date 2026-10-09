// Browser consumer check: serve through the desktop Solid Vite plugin and call
// checkSheetDialogName() from an installed browser runner. No CLI or account is used.
import { flush } from "solid-js";
import { render } from "@solidjs/web";
import { setLanguage } from "../l10n";
import { alert, presentSheet, Sheet, SheetHost } from "./overlay";

export async function checkSheetDialogName(): Promise<Record<string, string>> {
  const assert = (condition: unknown, message: string) => {
    if (!condition) throw new Error(message);
  };
  setLanguage("en");
  const root = document.createElement("div");
  root.id = "root";
  document.body.append(root);
  const dispose = render(() => <SheetHost />, root);
  const settle = async () => { await Promise.resolve(); flush(); await new Promise((r) => setTimeout(r, 0)); flush(); };
  const dialogs = () => [...document.querySelectorAll<HTMLElement>("[role=dialog]")];
  const nameOf = (dialog: HTMLElement) => {
    const id = dialog.getAttribute("aria-labelledby");
    return id ? document.getElementById(id)?.textContent?.trim() ?? "" : "";
  };
  try {
    const first = presentSheet((dismiss) => <Sheet title="First sheet" onCancel={dismiss} confirm="Done" onConfirm={dismiss} />);
    await settle();
    assert(dialogs().length === 1, "Sheet did not open");
    assert(nameOf(dialogs()[0]!) === "First sheet", "Sheet dialog has no accessible name");

    const second = presentSheet((dismiss) => <Sheet title="Second sheet" onCancel={dismiss} />);
    await settle();
    assert(dialogs().length === 2, "Stacked sheet did not open");
    assert(nameOf(dialogs()[0]!) === "First sheet" && nameOf(dialogs()[1]!) === "Second sheet", "Stacked sheets must each name their own dialog");
    const ids = dialogs().map((d) => d.getAttribute("aria-labelledby"));
    assert(new Set(ids).size === 2, "Stacked sheet title ids must be unique");

    second.dismiss();
    first.dismiss();
    await settle();
    assert(dialogs().length === 0, "Sheets did not dismiss");

    const answer = alert({ message: "Remove this thing?", buttons: [{ title: "Remove", destructive: true }, { title: "Cancel" }] });
    await settle();
    assert(dialogs().length === 1, "Alert did not open");
    assert(nameOf(dialogs()[0]!) === "Remove this thing?", "Alert dialog has no accessible name");
    dialogs()[0]!.querySelector<HTMLElement>(".alert")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    assert((await answer) === 1, "Escape must still answer Cancel");
    await settle();
    assert(dialogs().length === 0, "Alert did not dismiss on Escape");

    return { sheet: "named", stacked: "unique ids", alert: "named", escape: "cancels" };
  } finally {
    dispose();
    root.remove();
  }
}
