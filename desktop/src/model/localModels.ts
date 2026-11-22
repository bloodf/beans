// The Runner's local models as the Search pane lists them: the suggested one first, then those
// that suit the Runner's computer, each with what keeps it from the computer in words. The Runner
// decides what fits (`schedule_models::recommend`); this only words and orders it.

import { L } from "../l10n";
import * as Format from "./format";
import type { WireInstallProgress, WireLocalModel, WireModelPreview } from "./wire";

/** What keeps a model from the Runner's computer, or is worth knowing before downloading it.
 * `multilingual` and `installed` go unsaid: the languages and the menus show them; a reason
 * from a newer Runner is left out rather than shown raw. */
export function reasonText(reason: string): string | null {
  switch (reason) {
    case "needs_onnx_runtime":
      return L("Needs ONNX Runtime");
    case "not_enough_memory":
      return L("Needs more memory");
    case "not_enough_disk":
      return L("Not enough disk space");
    case "not_downloadable":
      return L("Not available to download");
    case "english_only":
      return L("English only");
    case "slow_here":
      return L("Slow on this computer");
    default:
      return null;
  }
}

/** A model's row text: whether it is the suggestion, its languages and size, then what to know. */
export function modelSummary(model: WireLocalModel, recommended: boolean): string {
  const notes = (model.fit?.reasons ?? []).map(reasonText).filter((text): text is string => text !== null);
  return [...(recommended ? [L("Recommended")] : []), model.languages, Format.megabytes(model.bytes), ...notes].join(" · ");
}

/** A model's tooltip: its license, and the memory it takes when the Runner says. */
export function modelTooltip(model: WireLocalModel): string {
  return model.fit ? L("%@ · about %@ of memory when loaded", model.license, Format.megabytes(model.fit.memory_bytes)) : model.license;
}

/** Whether the Runner can download `model`: it fits the computer. A Runner older than
 * suggestions says only whether it is pinned. */
export function canDownload(model: WireLocalModel): boolean {
  return model.fit ? model.fit.fits : model.pinned;
}

/** Models still to download, the suggested one first, then those that fit, each group in
 * catalog order. */
export function downloadOrder(models: WireLocalModel[], suggested: string | null | undefined): WireLocalModel[] {
  const rank = (model: WireLocalModel) => (model.id === suggested ? 0 : canDownload(model) ? 1 : 2);
  return models.filter((model) => !model.installed).sort((a, b) => rank(a) - rank(b));
}

/** What approving `preview` downloads to `runner`, and how it is checked, for the dialog. */
export function previewSummary(preview: WireModelPreview, runner: string): string {
  const files = L("%@ from %@ to %@, checked against its pinned SHA-256 before it runs. License: %@", Format.megabytes(preview.total_bytes), preview.hub ?? "Hugging Face", runner, preview.license);
  const runtime = preview.runtime_download;
  if (!runtime) return files;
  return `${files} ${L("It also downloads ONNX Runtime %@ (%@) from %@, which runs the model, checked the same way.", runtime.version, Format.megabytes(runtime.bytes), runtime.from)}`;
}

/** A download's progress for its row, when the Runner reports `modelID` under way. */
export function progressText(progress: WireInstallProgress | null, modelID: string): string | undefined {
  const installing = progress?.installing;
  if (!installing || installing.model_id !== modelID) return undefined;
  if (installing.total_bytes <= 0) return L("Downloading…");
  const percent = Math.min(100, Math.floor((installing.done_bytes / installing.total_bytes) * 100));
  return L("Downloading… %d%% (%@ of %@)", percent, Format.megabytes(installing.done_bytes), Format.megabytes(installing.total_bytes));
}
