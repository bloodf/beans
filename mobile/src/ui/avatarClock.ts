// One activation owner for a Reanimated frame callback. Motion off (including Reduce Motion)
// snaps to the target; it never opens a transition window or schedules a timer.

export interface AvatarVisibility {
  appActive: boolean;
  focused: boolean;
  reduceMotion: boolean;
  visible: boolean;
  motion: boolean;
}

export function ambientOn(v: AvatarVisibility): boolean {
  return v.visible && v.appActive && v.focused && !v.reduceMotion && v.motion;
}

export interface AvatarClock {
  update(visibility: AvatarVisibility): void;
  dispose(): void;
}

// The component supplies the real useFrameCallback.setActive; tests supply a counted host.
export function createAvatarClock(setRunning: (running: boolean) => void): AvatarClock {
  let running = false;
  let disposed = false;
  return {
    update(visibility) {
      if (disposed) return;
      const next = ambientOn(visibility);
      if (next === running) return;
      running = next;
      setRunning(next);
    },
    dispose() {
      if (disposed) return;
      if (running) setRunning(false);
      running = false;
      disposed = true;
    },
  };
}
