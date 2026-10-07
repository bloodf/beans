import type { AvatarGeometry } from "@beans/blobatar";
import { avatarFrame, avatarMorphProgress, interpolateAvatarGeometry } from "@beans/blobatar/frame";

/** Retargets from the last displayed numeric geometry, never an obsolete endpoint. */
export class AvatarMotion {
  private from: AvatarGeometry;
  private target: AvatarGeometry;
  private displayed: AvatarGeometry;
  private startedAt = 0;
  private transitioning = false;
  constructor(initial: AvatarGeometry) {
    this.from = this.target = this.displayed = initial;
  }

  retarget(target: AvatarGeometry, now: number, animate: boolean): void {
    if (this.target === target) return;
    this.from = this.displayed;
    this.target = target;
    this.startedAt = now;
    this.transitioning = animate && target.motion;
    if (!this.transitioning) this.from = this.displayed = target;
  }

  frame(now: number, animate: boolean) {
    if (!animate || !this.target.motion) {
      this.displayed = this.target;
      this.transitioning = false;
    } else if (this.transitioning) {
      const progress = avatarMorphProgress(now - this.startedAt, this.target);
      this.displayed = progress.done ? this.target : interpolateAvatarGeometry(this.from, this.target, progress.t);
      this.transitioning = !progress.done;
    }
    return avatarFrame(this.displayed, now, animate && this.target.motion ? 1 : 0);
  }
}
