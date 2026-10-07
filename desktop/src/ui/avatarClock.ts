export interface AvatarScheduler {
  request(callback: FrameRequestCallback): number;
  cancel(handle: number): void;
}

/** A single frame request, only while at least one visible moving avatar subscribes. */
export class AvatarClock {
  private listeners = new Set<FrameRequestCallback>();
  private handle: number | undefined;
  constructor(private scheduler: AvatarScheduler) {}

  subscribe(listener: FrameRequestCallback): () => void {
    this.listeners.add(listener);
    this.schedule();
    return () => {
      this.listeners.delete(listener);
      if (!this.listeners.size && this.handle !== undefined) {
        this.scheduler.cancel(this.handle);
        this.handle = undefined;
      }
    };
  }

  private schedule(): void {
    if (this.listeners.size && this.handle === undefined) this.handle = this.scheduler.request(this.tick);
  }

  private tick = (time: number) => {
    this.handle = undefined;
    for (const listener of this.listeners) listener(time);
    this.schedule();
  };
}

export const avatarClock = new AvatarClock({
  request: (callback) => window.requestAnimationFrame(callback),
  cancel: (handle) => window.cancelAnimationFrame(handle),
});
