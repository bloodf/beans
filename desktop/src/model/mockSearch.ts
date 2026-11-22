// Settings › Search and the palette's Related and Pictures sections in the seeded demo
// (`BEANS_MOCK=1`, `?mock=1`), which has no CLI: a Runner that searches with a local text model
// and an image model, a catalog with a suggestion and a model that does not fit, and meaning
// matches over the demo's chats. Nothing here downloads or installs (issue #586).

import type { Chat } from "./models";
import type { WireLocalModels, WireModelPreview, WireRelatedHit, WireRelatedResults, WireSearchIndexStatus, WireTextSource } from "./wire";

/** What the demo Runner searches with: both roles chosen, ready, and caught up. */
export interface MockSearchChoice {
  text: WireTextSource | null;
  image: string | null;
}

export function mockSearchChoice(): MockSearchChoice {
  return { text: { kind: "local", model_id: "multilingual-e5-small" }, image: "clip-vit-b32" };
}

export function mockSearchIndexStatus(choice: MockSearchChoice): WireSearchIndexStatus {
  return {
    text: { choice: choice.text, state: choice.text ? "ready" : "off", error: null, indexed: choice.text ? 1284 : 0, pending: 0 },
    image: { choice: choice.image, state: choice.image ? "ready" : "off", error: null, indexed: choice.image ? 37 : 0, pending: 0 },
    describe: { choice: null, pending: 0 },
    profiles: [],
    runs_models: true,
  };
}

/** The catalog as the demo Runner sees it: installed, suggested, fits, and does not fit. */
export function mockLocalModels(choice: MockSearchChoice): WireLocalModels {
  const fits = (memory: number, reasons: string[] = []) => ({ fits: true, reasons, memory_bytes: memory });
  return {
    max_bytes: 1_000_000_000,
    models: [
      { id: "multilingual-e5-small", name: "Multilingual E5 Small", kind: "text", languages: "About 100 languages", license: "MIT", installed: choice.text?.kind === "local", pinned: true, bytes: 135_000_000, fit: fits(420_000_000, ["multilingual", "installed"]) },
      { id: "bge-small-en", name: "BGE Small (English)", kind: "text", languages: "English", license: "MIT", installed: false, pinned: true, bytes: 35_000_000, fit: fits(240_000_000, ["english_only"]) },
      { id: "bge-m3", name: "BGE M3", kind: "text", languages: "About 100 languages", license: "MIT", installed: false, pinned: true, bytes: 2_270_000_000, fit: { fits: false, reasons: ["not_enough_memory"], memory_bytes: 9_000_000_000 } },
      { id: "clip-vit-b32", name: "CLIP ViT-B/32", kind: "image", languages: "English", license: "MIT", installed: choice.image !== null, pinned: true, bytes: 605_000_000, fit: fits(1_100_000_000, ["installed"]) },
      { id: "siglip-base", name: "SigLIP Base", kind: "image", languages: "English", license: "Apache-2.0", installed: false, pinned: true, bytes: 813_000_000, fit: fits(1_600_000_000, ["slow_here"]) },
    ],
    recommended: { text: "multilingual-e5-small", image: "siglip-base" },
    machine: { memory_bytes: 16_000_000_000, free_disk_bytes: 210_000_000_000, cores: 8, os: "linux", arch: "x86_64", onnx_runtime: true },
    needs: { multilingual: true, images: true },
  };
}

/** What a download would take; the demo shows it and never installs. */
export function mockModelPreview(models: WireLocalModels, modelID: string): WireModelPreview {
  const model = models.models.find((each) => each.id === modelID);
  return {
    preview_token: `demo-${modelID}`,
    preview_digest: "demo",
    name: model?.name ?? modelID,
    license: model?.license ?? "MIT",
    total_bytes: model?.bytes ?? 0,
    hub: "Hugging Face",
    runtime_download: null,
  };
}

/** Words that mean about the same thing in the demo's chats, as a text model would place them. */
const CONCEPTS: string[][] = [
  ["launch", "release", "ship", "rollout", "announcement", "go/no-go"],
  ["setup", "onboarding", "pairing", "getting started", "first steps", "welcome", "new user"],
  ["website", "site", "download", "link", "docs"],
  ["brief", "summary", "morning", "report", "checklist", "blockers"],
  ["relay", "server", "health", "disk", "offline", "uptime"],
  ["phone", "iphone", "mobile"],
];

const PICTURES: { chatID: string; name: string; snippet: string; concepts: number[] }[] = [
  { chatID: "chat-launch", name: "launch-banner.png", snippet: "A banner with the Beans logo over a sunrise", concepts: [0] },
  { chatID: "chat-scout", name: "pairing-screen.png", snippet: "A phone showing the pairing screen and a QR code", concepts: [1, 5] },
];

function conceptsIn(text: string): Set<number> {
  const folded = text.toLowerCase();
  return new Set(CONCEPTS.flatMap((words, index) => (words.some((word) => folded.includes(word)) ? [index] : [])));
}

/** Meaning matches over the demo's chats, best first: messages sharing a concept with the query. */
export function mockRelated(query: string, chats: Chat[], choice: MockSearchChoice): WireRelatedResults {
  const wanted = conceptsIn(query);
  const related: WireRelatedHit[] = [];
  if (choice.text && wanted.size > 0) {
    for (const chat of chats) {
      for (const message of chat.messages) {
        if (message.body.kind !== "text") continue;
        const shared = [...conceptsIn(message.body.text)].filter((concept) => wanted.has(concept)).length;
        if (shared === 0) continue;
        const author = message.author.kind === "bot" ? { kind: "bot", bot_id: message.author.botID } : { kind: message.author.kind };
        const snippet = message.body.text.replace(/\s+/g, " ").slice(0, 140);
        related.push({ kind: "message", chat_id: chat.id, message_id: message.id, snippet, author, created_at: message.createdAt / 1000, score: shared / wanted.size });
      }
    }
  }
  related.sort((a, b) => b.score - a.score || b.created_at - a.created_at);
  const images: WireRelatedHit[] = choice.image
    ? PICTURES.filter((picture) => picture.concepts.some((concept) => wanted.has(concept))).map((picture, index) => ({
        kind: "image", chat_id: picture.chatID, message_id: `msg-demo-picture-${index}`, attachment_id: `att-demo-picture-${index}`,
        name: picture.name, snippet: picture.snippet, author: { kind: "you" }, created_at: 0, score: 0.5,
      }))
    : [];
  return {
    state: choice.text ? "ready" : "off",
    related: related.slice(0, 8),
    image_state: choice.image ? "ready" : "off",
    images,
    runner_id: "dev-workbench",
  };
}
