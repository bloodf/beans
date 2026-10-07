import { createContext, useContext, useSyncExternalStore, type ReactNode } from "react";

const AvatarVisible = createContext(true);

// FlashList keeps draw-distance/recycled cells mounted. Only rows it reports viewable may tick.
export class AvatarViewability {
  private visible = new Set<string>();
  private listeners = new Set<() => void>();

  update(keys: Iterable<string>) {
    const next = new Set(keys);
    if (next.size === this.visible.size && [...next].every((key) => this.visible.has(key))) return;
    this.visible = next;
    for (const notify of this.listeners) notify();
  }

  has(key: string) { return this.visible.has(key); }

  subscribe = (notify: () => void) => {
    this.listeners.add(notify);
    return () => { this.listeners.delete(notify); };
  };
}

export function AvatarRowVisibility({ tracker, rowKey, children }: { tracker: AvatarViewability; rowKey: string; children: ReactNode }) {
  const visible = useSyncExternalStore(tracker.subscribe, () => tracker.has(rowKey), () => false);
  return <AvatarVisible.Provider value={visible}>{children}</AvatarVisible.Provider>;
}

export function useAvatarRowVisible() { return useContext(AvatarVisible); }

// Nonvirtualized scrolling forms also retain offscreen avatars. Their scroll events request a
// native measurement; only a visibility transition updates React, never an avatar animation frame.
const scrollListeners = new Set<() => void>();
export function notifyAvatarScroll() {
  for (const check of scrollListeners) check();
}
export function subscribeAvatarScroll(check: () => void) {
  scrollListeners.add(check);
  return () => { scrollListeners.delete(check); };
}

export function avatarIntersectsViewport(x: number, y: number, width: number, height: number, viewportWidth: number, viewportHeight: number) {
  return width > 0 && height > 0 && x < viewportWidth && y < viewportHeight && x + width > 0 && y + height > 0;
}
