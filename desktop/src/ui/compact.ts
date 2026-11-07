// The inspector's Compact, per chat: under way until the Runner answers, then done for a moment,
// or why it did not compact in a toast at the foot of the pane.

/** What a chat's last Compact is doing: nothing, under way, or done and checked off. */
export type CompactPhase = "idle" | "running" | "done";

/** How long the done check and the toast stay up. */
export const compactShown = 2000;

/** The timers a Compact waits on, so a test can move time on its own. */
export interface CompactScheduler {
  setTimeout(handler: () => void, ms: number): number;
  clearTimeout(handle: number): void;
}

/** The app's own timers. */
export const browserScheduler: CompactScheduler = {
  setTimeout: (handler, ms) => window.setTimeout(handler, ms),
  clearTimeout: (handle) => window.clearTimeout(handle),
};

/** The Compact of each chat the inspector has asked for. `changed` is called whenever a chat's
 * Compact or the toast moves, so the pane can redraw. */
export class Compacts {
  private phases = new Map<string, CompactPhase>();
  private timers = new Map<string, number>();
  private toastTimer: number | undefined;
  private toastText = "";

  constructor(private scheduler: CompactScheduler, private changed: () => void) {}

  /** The chat's Compact. */
  phase(chatID: string): CompactPhase {
    return this.phases.get(chatID) ?? "idle";
  }

  /** Why a Compact did not compact, at the foot of the pane. Empty when there is none. */
  toast(): string {
    return this.toastText;
  }

  /** Starts a Compact. False when one is already under way, so a second click sends nothing. */
  start(chatID: string): boolean {
    if (this.phase(chatID) === "running") return false;
    this.forget(chatID);
    this.phases.set(chatID, "running");
    this.changed();
    return true;
  }

  /** The Runner compacted it: the row checks it off for a moment. */
  succeed(chatID: string): void {
    // A reply for a Compact that is not under way is late, and changes nothing.
    if (this.phase(chatID) !== "running") return;
    this.phases.set(chatID, "done");
    this.timers.set(chatID, this.scheduler.setTimeout(() => {
      this.timers.delete(chatID);
      if (this.phase(chatID) === "done") this.phases.delete(chatID);
      this.changed();
    }, compactShown));
    this.changed();
  }

  /** It did not: the toast says why, and the row goes back to the context size. */
  fail(chatID: string, message: string): void {
    this.forget(chatID);
    this.phases.delete(chatID);
    this.toastText = message;
    if (this.toastTimer !== undefined) this.scheduler.clearTimeout(this.toastTimer);
    this.toastTimer = this.scheduler.setTimeout(() => {
      this.toastTimer = undefined;
      this.toastText = "";
      this.changed();
    }, compactShown);
    this.changed();
  }

  /** A chat's pending timer, so a late one cannot undo a newer Compact. */
  private forget(chatID: string): void {
    const timer = this.timers.get(chatID);
    if (timer === undefined) return;
    this.scheduler.clearTimeout(timer);
    this.timers.delete(chatID);
  }
}
