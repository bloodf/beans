// The Search pane's model list: the suggestion first, what fits next, and each row's words.

import { expect, test } from "vitest";
import { canDownload, downloadOrder, modelSummary, modelTooltip, previewSummary, progressText, reasonText } from "./localModels";
import type { WireLocalModel, WireModelPreview } from "./wire";

function model(id: string, fields: Partial<WireLocalModel> = {}): WireLocalModel {
  return { id, name: id, kind: "text", languages: "English", license: "MIT", installed: false, pinned: true, bytes: 30_000_000, fit: { fits: true, reasons: [], memory_bytes: 36_000_000 }, ...fields };
}

const unfit = (reasons: string[]) => ({ fits: false, reasons, memory_bytes: 1 });

test("the suggestion leads, then what fits, then the rest, each in catalog order", () => {
  const models = [
    model("big", { fit: unfit(["not_enough_memory"]) }),
    model("small"),
    model("installed", { installed: true }),
    model("best"),
    model("unpinned", { fit: unfit(["not_downloadable"]) }),
    model("medium"),
  ];
  expect(downloadOrder(models, "best").map((m) => m.id)).toEqual(["best", "small", "medium", "big", "unpinned"]);
  expect(downloadOrder(models, null).map((m) => m.id)).toEqual(["small", "best", "medium", "big", "unpinned"]);
  expect(downloadOrder(models, "installed").map((m) => m.id), "an installed suggestion is not listed to download").toEqual(["small", "best", "medium", "big", "unpinned"]);
  expect(downloadOrder(models, "no-such-model").map((m) => m.id)).toEqual(["small", "best", "medium", "big", "unpinned"]);
  expect(downloadOrder([], "best")).toEqual([]);
  expect(models.map((m) => m.id), "the catalog itself is left as it was").toEqual(["big", "small", "installed", "best", "unpinned", "medium"]);
});

test("only what fits downloads; an older Runner's pinned models still do", () => {
  expect(canDownload(model("fits"))).toBe(true);
  expect(canDownload(model("too-big", { fit: unfit(["not_enough_disk"]) }))).toBe(false);
  expect(canDownload(model("old-pinned", { fit: undefined }))).toBe(true);
  expect(canDownload(model("old-unpinned", { fit: undefined, pinned: false }))).toBe(false);
  expect(canDownload(model("unpinned-but-fits", { pinned: false })), "the Runner's fit decides").toBe(true);
});

test("rows say the suggestion, the languages, the size, and what stands in the way", () => {
  expect(modelSummary(model("m"), true)).toBe("Recommended · English · 30 MB");
  expect(modelSummary(model("m"), false)).toBe("English · 30 MB");
  const blocked = model("m", { languages: "100+ languages", bytes: 1_200_000_000, fit: unfit(["needs_onnx_runtime", "not_enough_memory", "multilingual", "slow_here"]) });
  expect(modelSummary(blocked, false)).toBe("100+ languages · 1.2 GB · Needs ONNX Runtime · Needs more memory · Slow on this computer");
  expect(modelSummary(model("m", { fit: undefined }), false), "an older Runner says no reasons").toBe("English · 30 MB");
  expect(modelSummary(model("m", { fit: unfit(["some_future_reason", "installed"]) }), false), "unknown and unsaid reasons are left out").toBe("English · 30 MB");
  for (const reason of ["needs_onnx_runtime", "not_enough_memory", "not_enough_disk", "not_downloadable", "english_only", "slow_here"]) {
    expect(reasonText(reason), reason).toBeTruthy();
  }
  expect(reasonText("multilingual")).toBeNull();
});

test("tooltips add the memory a model takes when the Runner knows it", () => {
  expect(modelTooltip(model("m"))).toBe("MIT · about 36 MB of memory when loaded");
  expect(modelTooltip(model("m", { fit: undefined }))).toBe("MIT");
});

test("the download dialog names the hub, and the runtime when one comes with the model", () => {
  const preview: WireModelPreview = { preview_token: "t", preview_digest: "d", name: "Model", license: "MIT", total_bytes: 35_000_000, hub: "Hugging Face" };
  expect(previewSummary(preview, "Studio")).toBe("35 MB from Hugging Face to Studio, checked against its pinned SHA-256 before it runs. License: MIT");
  expect(previewSummary({ ...preview, hub: "models.example.com" }, "Studio")).toContain("from models.example.com to Studio");
  expect(previewSummary({ ...preview, hub: undefined }, "Studio"), "an older Runner names no hub").toContain("from Hugging Face");
  const withRuntime = previewSummary({ ...preview, runtime_download: { version: "1.31.1", bytes: 23_760_988, from: "files.pythonhosted.org" } }, "Studio");
  expect(withRuntime).toBe(
    "35 MB from Hugging Face to Studio, checked against its pinned SHA-256 before it runs. License: MIT It also downloads ONNX Runtime 1.31.1 (24 MB) from files.pythonhosted.org, which runs the model, checked the same way.",
  );
  expect(previewSummary({ ...preview, runtime_download: null }, "Studio")).not.toContain("ONNX Runtime");
});

// Issue #581: a download's row says how far it has come while the Runner reports it.
test("a download under way reads as its percent and bytes", () => {
  const progress = (model_id: string, done_bytes: number, total_bytes: number) => ({ installing: { model_id, done_bytes, total_bytes } });
  expect(progressText(progress("m", 47_000_000, 135_000_000), "m")).toBe("Downloading… 34% (47 MB of 135 MB)");
  expect(progressText(progress("m", 135_000_000, 135_000_000), "m")).toBe("Downloading… 100% (135 MB of 135 MB)");
  expect(progressText(progress("m", 0, 0), "m")).toBe("Downloading…");
  expect(progressText(progress("other", 1, 2), "m"), "another model's download").toBeUndefined();
  expect(progressText({ installing: null }, "m")).toBeUndefined();
  expect(progressText(null, "m")).toBeUndefined();
});
