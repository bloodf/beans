import { watchWindowState } from "../host";

interface VisibilityEntry {
  intersects: boolean;
  change: (visible: boolean, reducedMotion: boolean) => void;
}
const entries = new Map<Element, VisibilityEntry>();
let observer: IntersectionObserver | undefined;
let media: MediaQueryList | undefined;
let unwatchWindow: (() => void) | undefined;
let windowVisible = true;

function report(): void {
  const visible = document.visibilityState === "visible" && windowVisible;
  for (const entry of entries.values()) entry.change(visible && entry.intersects, media!.matches);
}

/** One observer and document/native-window listeners, released with the final avatar. */
export function watchAvatarVisibility(element: Element, change: VisibilityEntry["change"]): () => void {
  entries.set(element, { intersects: false, change });
  if (!observer) {
    media = window.matchMedia("(prefers-reduced-motion: reduce)");
    media.addEventListener("change", report);
    document.addEventListener("visibilitychange", report);
    observer = new IntersectionObserver((changes) => {
      for (const entry of changes) {
        const found = entries.get(entry.target);
        if (found) found.intersects = entry.isIntersecting && entry.intersectionRatio > 0;
      }
      report();
    });
    unwatchWindow = watchWindowState((state) => {
      windowVisible = state.visible && !state.minimized && state.focused;
      report();
    });
  }
  observer.observe(element);
  change(false, media!.matches);
  return () => {
    observer!.unobserve(element);
    entries.delete(element);
    if (!entries.size) {
      observer!.disconnect(); observer = undefined;
      document.removeEventListener("visibilitychange", report);
      media!.removeEventListener("change", report); media = undefined;
      unwatchWindow?.(); unwatchWindow = undefined;
      windowVisible = true;
    }
  };
}
