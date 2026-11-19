// Browser consumer check: Settings › Search lists the Runner's suggestion first, offers to
// download only what fits, and downloads nothing until the user approves the preview, which
// names the hub and the runtime that comes with the model. The store's Runner calls are stubbed;
// no CLI or account is used.
import { flush } from "solid-js";
import { render } from "@solidjs/web";
import { setLanguage } from "../../l10n";
import type { Device } from "../../model/models";
import { store } from "../../model/store";
import type { WireLocalModel, WireLocalModels, WireModelPreview, WireSearchIndexStatus } from "../../model/wire";
import { SheetHost } from "../overlay";
import { SearchSettings } from "./searchPane";

export async function checkSearchSuggestion(): Promise<Record<string, string>> {
  const assert = (condition: unknown, message: string) => {
    if (!condition) throw new Error(message);
  };
  setLanguage("en");
  const root = document.createElement("div");
  root.id = "root";
  document.body.append(root);
  const role = { choice: null, state: "off", error: null, indexed: 0, pending: 0 };
  const status: WireSearchIndexStatus = { text: role, image: role, describe: { choice: null, pending: 0 }, profiles: [], runs_models: true };
  const model = (id: string, name: string, fields: Partial<WireLocalModel>): WireLocalModel => ({
    id, name, kind: "text", languages: "English", license: "MIT", installed: false, pinned: true, bytes: 35_000_000,
    fit: { fits: true, reasons: [], memory_bytes: 240_000_000 }, ...fields,
  });
  let listing: WireLocalModels = {
    max_bytes: 1_000_000_000,
    models: [
      model("small", "Small model", {}),
      model("big", "Big model", { fit: { fits: false, reasons: ["not_enough_memory"], memory_bytes: 9_000_000_000 } }),
      model("best", "Best model", { languages: "About 100 languages", fit: { fits: true, reasons: ["multilingual"], memory_bytes: 400_000_000 } }),
    ],
    recommended: { text: "best", image: null },
  };
  const preview: WireModelPreview = {
    preview_token: "token", preview_digest: "digest", name: "Best model", license: "MIT", total_bytes: 135_000_000, hub: "Hugging Face",
    runtime_download: { version: "1.31.1", bytes: 23_760_988, from: "files.pythonhosted.org" },
  };
  const calls: string[] = [];
  const saved = {
    searchIndexStatus: store.searchIndexStatus, localModels: store.localModels,
    previewLocalModel: store.previewLocalModel, installLocalModel: store.installLocalModel,
  };
  store.searchIndexStatus = async () => status;
  store.localModels = async () => { calls.push("list"); return listing; };
  store.previewLocalModel = async (_runner, id) => { calls.push(`preview ${id}`); return preview; };
  store.installLocalModel = async (_runner, approved) => {
    calls.push(`install ${approved.preview_token}`);
    listing = { ...listing, models: listing.models.map((m) => (m.id === "best" ? { ...m, installed: true } : m)) };
  };
  const device = { id: "runner", name: "Studio", os: "linux", plugins: [] } as unknown as Device;
  const dispose = render(() => (<><SearchSettings device={device} /><SheetHost /></>), root);
  const settle = async () => { for (let i = 0; i < 4; i++) { await Promise.resolve(); flush(); await new Promise((r) => setTimeout(r, 0)); flush(); } };
  const rows = () => [...root.querySelectorAll<HTMLElement>(".action-row")];
  const row = (name: string) => rows().find((r) => r.dataset.label === name);
  const download = (name: string) => [...(row(name)?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((b) => b.textContent?.includes("Download"));
  const dialog = () => document.querySelector<HTMLElement>("[role=dialog]");
  try {
    await settle();
    assert(root.textContent?.includes("Suggested for Studio: Best model. Nothing downloads until you approve it."), "The suggestion is not said");
    const order = rows().map((r) => r.dataset.label);
    assert(order.join(",") === "Best model,Small model,Big model", `Suggestion first, then what fits: ${order.join(",")}`);
    assert(row("Best model")?.textContent?.includes("Recommended · About 100 languages · 35 MB"), "The suggestion's row does not say so");
    assert(row("Big model")?.textContent?.includes("Needs more memory"), "Why a model does not fit is not said");
    assert(!download("Big model"), "A model that does not fit offers a download");
    assert(download("Small model"), "A model that fits offers no download");

    download("Best model")!.click();
    await settle();
    assert(dialog()?.textContent?.includes("from Hugging Face to Studio"), "The approval does not name the hub");
    assert(dialog()?.textContent?.includes("It also downloads ONNX Runtime 1.31.1 (24 MB) from files.pythonhosted.org"), "The approval does not name the runtime");
    dialog()!.querySelector<HTMLElement>(".alert")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();
    assert(!calls.some((c) => c.startsWith("install")), "Cancelling the approval still installed");
    assert(!dialog(), "The approval did not close");

    download("Best model")!.click();
    await settle();
    dialog()!.querySelector<HTMLElement>(".alert")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();
    assert(calls.includes("install token"), `Approving did not install: ${calls.join(", ")}`);
    assert(!row("Best model") && !root.textContent?.includes("Suggested for Studio"), "An installed suggestion is still offered");
    return { order: order.join(" › "), approval: "hub and runtime named; cancel installs nothing; approve installs once" };
  } finally {
    Object.assign(store, saved);
    dispose();
    root.remove();
  }
}
