// The phone's door to the Rust core: start it once, then the JSON API the desktop app speaks
// to the CLI (`request`) and the events the CLI would push over the websocket (`onEvent`).
import { requireNativeModule, type EventSubscription } from "expo-modules-core";

interface Native {
  start(home: string, name: string, os: string, osVersion: string, model: string): void;
  request(method: string, params: string): Promise<string>;
  wake(): void;
  /// Android only.
  setOpenChat?(chatId: string | null): void;
  updateCapability?(): "supported" | "development" | "store_managed" | "native_verifier_unavailable";
  updateResult?(): string | null;
  checkUpdate?(): Promise<UpdateOffer | null>;
  installUpdate?(id: string): Promise<void>;
  cancelUpdate?(): "cancelled" | "committed";
  addListener(event: "updateProgress", listener: (payload: UpdateProgress) => void): EventSubscription;
  addListener(event: "event", listener: (payload: { json: string }) => void): EventSubscription;
}

const native = requireNativeModule<Native>("BeansCore");
// Creating the module also prunes abandoned paste files once per native process, before
// new copies. Keep filesystem ownership checks in the native module that writes them.
const pasteCache = requireNativeModule<{ deleteFiles(uris: string[]): Promise<void> }>("ImagePasteView");

export function deletePastedImages(uris: string[]): Promise<void> {
  return pasteCache.deleteFiles(uris);
}

export interface HostFacts {
  name: string;
  os: string;
  os_version: string;
  model: string;
}

/// One event frame, as the websocket carries it.
export interface Frame {
  event: string;
  data: unknown;
}

export function start(home: string, facts: HostFacts) {
  native.start(home, facts.name, facts.os, facts.os_version, facts.model);
}

/// One API call. Throws with the core's message when it answers with an error.
export async function request<T = unknown>(method: string, params: unknown = {}): Promise<T> {
  const raw = await native.request(method, JSON.stringify(params ?? {}));
  const parsed = JSON.parse(raw) as { result?: T; error?: { message: string } };
  if (parsed.error) throw new Error(parsed.error.message);
  return parsed.result as T;
}

export function onEvent(listener: (frame: Frame) => void): () => void {
  const subscription = native.addListener("event", ({ json }) => listener(JSON.parse(json) as Frame));
  return () => subscription.remove();
}

/// The app came to the foreground: sync now rather than after the backoff.
export function wake() {
  native.wake();
}

/// Android: the chat on screen, or null. While the app is in front, the native push service
/// posts nothing for it and clears what it posted for it. iOS asks the notification handler.
export function setOpenChat(chatId: string | null) {
  native.setOpenChat?.(chatId);
}

export interface UpdateOffer { id: string; version: string; notes: string }
export interface UpdateProgress { id: string; phase: "downloading" | "installing"; downloaded: number; total: number }
export const androidUpdates = {
  result: () => native.updateResult?.() ?? null,
  onProgress: (listener: (payload: UpdateProgress) => void) => {
    const subscription = native.addListener("updateProgress", listener);
    return () => subscription.remove();
  },
  capability: () => {
    if (![native.updateCapability, native.updateResult, native.checkUpdate, native.installUpdate, native.cancelUpdate, native.addListener].every(fn => typeof fn === "function")) return "native_verifier_unavailable" as const;
    try { return native.updateCapability!(); } catch { return "native_verifier_unavailable" as const; }
  },
  check: async () => {
    if (!native.checkUpdate) throw new Error("Native update verifier unavailable");
    return native.checkUpdate();
  },
  install: async (id: string) => {
    if (!native.installUpdate) throw new Error("Native installer unavailable");
    await native.installUpdate(id);
  },
  cancel: () => native.cancelUpdate?.() ?? "cancelled",
};
