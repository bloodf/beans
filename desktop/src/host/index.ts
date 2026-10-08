// The app around the page: the Go side's services when the page runs in a Beans window, and
// stand-ins when it runs in a browser tab of the dev server (for working on the views), where it
// talks to a CLI's websocket itself or runs the demo.

import { isMyGo, onFileDrop, type FileDrop } from "mygo-runtime";
import { L } from "../l10n";
import { browserCLIPort, requireCLIPort } from "./cliPort";
import {
  CLI,
  events,
  Files,
  Host,
  Menus,
  Notices,
  Prefs,
  type CLIState,
  type ChooseOptions,
  type FileInfo,
  type HostInfo,
  type MenuItemSpec,
  type MenuItemState,
  type NoticeOptions,
  type Preferences,
  type PreferencesPatch,
  type UpdaterState,
  type WindowState,
} from "../mygo";

export type { CLIState, FileInfo, HostInfo, MenuItemSpec, MenuItemState, Preferences, UpdaterState, WindowState };

export const inApp = isMyGo();

const query = new URLSearchParams(location.search);

/** A browser tab's system, which `?platform=windows` or `linux` overrides to preview theirs. */
function browserPlatform(): string {
  const posed = query.get("platform");
  if (posed === "windows" || posed === "linux" || posed === "darwin") return posed;
  const agent = navigator.userAgent;
  if (agent.includes("Windows")) return "windows";
  if (agent.includes("Mac")) return "darwin";
  return "linux";
}

let info: HostInfo = {
  platform: browserPlatform(),
  name: "Beans Dev",
  version: "dev",
  isDevelopment: true,
  isMock: query.get("mock") === "1",
  cliCommand: "beans serve --home ~/.beans-dev-v2 --port 4875",
  defaultCLIPort: 4875,
  productionRelayURL: "",
  locale: navigator.language,
  updatesEnabled: false,
  cliLogPath: "",
  keepsRunning: false,
};

const defaultPrefs: Preferences = {
  hadIdentity: false,
  selection: "",
  showsInspector: true,
  sendOnReturn: true,
  showTimestamps: true,
  relayURL: "",
  cliPort: 4875,
  appLanguage: "",
  appearance: "",
  sidebarWidth: 0,
  inspectorWidth: 0,
  sidebarCollapsed: false,
};

let prefs: Preferences = defaultPrefs;
let portFailure: Error | null = null;
let hostLoaded = false;
const prefsListeners = new Set<(prefs: Preferences) => void>();

/** Loads what the page needs before it renders: the build and the preferences. */
export async function loadHost(): Promise<void> {
  hostLoaded = false;
  portFailure = null;
  if (inApp) {
    [info, prefs] = await Promise.all([Host.info(), Prefs.all()]);
    events.prefsChanged.on((next) => {
      prefs = next;
      for (const listener of prefsListeners) listener(next);
    });
  } else {
    try {
      prefs = { ...defaultPrefs, ...JSON.parse(localStorage.getItem("beans-v2.prefs") ?? "{}") };
    } catch (error) {
      portFailure = error instanceof Error ? error : new Error(String(error));
      prefs = defaultPrefs;
    }
  }
  // Validate saved and explicit query ports independently.
  try {
    if (inApp) requireCLIPort(prefs.cliPort, info.defaultCLIPort);
    else browserCLIPort(prefs.cliPort, query, info.defaultCLIPort);
  } catch (error) {
    portFailure = error instanceof Error ? error : new Error(String(error));
  }
  hostLoaded = true;
  if (!inApp) transport?.reconnect();
}

export function hostInfo(): HostInfo {
  return info;
}

export function isWindows(): boolean {
  return info.platform === "windows";
}

export function isMacOS(): boolean {
  return info.platform === "darwin";
}

/** Who draws the main window's title bar: its panes' headers, around the traffic lights on macOS
 * (`mac`), and elsewhere under the window controls MyGo puts in a top corner (`overlay`), whose
 * room the `--mygo-titlebar-inset-*` variables give; a browser tab has the browser's (`system`). */
export function frameStyle(): "mac" | "overlay" | "system" {
  if (!inApp) return "system";
  return info.platform === "darwin" ? "mac" : "overlay";
}

export function preferences(): Preferences {
  return prefs;
}

export async function setPreferences(patch: PreferencesPatch): Promise<Preferences> {
  if (patch.cliPort !== undefined) {
    requireCLIPort(patch.cliPort, info.defaultCLIPort);
    if (!inApp) browserCLIPort(patch.cliPort, query, info.defaultCLIPort);
  } else if (portFailure) {
    throw portFailure;
  }
  if (inApp) {
    const next = await Prefs.set(patch);
    requireCLIPort(next.cliPort, info.defaultCLIPort);
    prefs = next;
  } else {
    prefs = { ...prefs, ...patch };
    try {
      localStorage.setItem("beans-v2.prefs", JSON.stringify(prefs));
    } catch {}
    for (const listener of prefsListeners) listener(prefs);
  }
  if (patch.cliPort !== undefined) {
    portFailure = null;
    if (!inApp) transport?.reconnect();
  }
  return prefs;
}

export function onPreferencesChanged(listener: (prefs: Preferences) => void): () => void {
  prefsListeners.add(listener);
  return () => prefsListeners.delete(listener);
}

// MARK: - The CLI

/** The connection to the CLI: the app's, through the Go side, or a websocket of the page's own
 * in a browser tab. */
export interface Transport {
  state(): Promise<CLIState>;
  request(method: string, params?: Record<string, unknown>): Promise<unknown>;
  reconnect(): void;
  onEvent(listener: (name: string, data: unknown) => void): () => void;
  onState(listener: (state: CLIState) => void): () => void;
}

class AppTransport implements Transport {
  state() {
    return CLI.state();
  }
  request(method: string, params: Record<string, unknown> = {}) {
    return CLI.request(method, params);
  }
  reconnect() {
    try {
      requireCLIPort(prefs.cliPort, info.defaultCLIPort);
      void CLI.reconnect().catch((error) => console.error("CLI reconnect failed:", error));
    } catch (error) {
      console.error("CLI port rejected:", error);
    }
  }
  onEvent(listener: (name: string, data: unknown) => void) {
    return events.cliEvent.on((frame) => {
      const { event, data } = frame as { event: string; data: unknown };
      listener(event, data);
    });
  }
  onState(listener: (state: CLIState) => void) {
    return events.cliState.on(listener);
  }
}

/** A browser tab's own websocket to a CLI started by hand (`beans serve`). The CLI must
 * explicitly allow this page's origin via BEANS_ALLOWED_ORIGINS (the desktop dev loop sets it). */
class SocketTransport implements Transport {
  private socket: WebSocket | null = null;
  private pending = new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();
  private nextID = 1;
  private delay = 400;
  private connection: CLIState["connection"] = "disconnected";
  private failure: Error | null = null;
  private eventListeners = new Set<(name: string, data: unknown) => void>();
  private stateListeners = new Set<(state: CLIState) => void>();

  constructor() {
    this.open();
  }

  private current(): CLIState {
    return {
      connection: this.connection,
      launcher: this.failure
        ? { kind: "failed", failure: { kind: "configuration", reason: this.failure.message } }
        : this.connection === "connected" ? { kind: "running", external: true } : { kind: "probing" },
      starting: false,
    };
  }

  private publish() {
    const state = this.current();
    for (const listener of this.stateListeners) listener(state);
  }

  private open() {
    let port: number;
    try {
      if (!hostLoaded) throw new Error(L("Wait for the app configuration before connecting to the CLI."));
      if (portFailure) throw portFailure;
      port = browserCLIPort(prefs.cliPort, query, info.defaultCLIPort);
    } catch (error) {
      this.failure = error instanceof Error ? error : new Error(String(error));
      this.connection = "disconnected";
      this.publish();
      return;
    }
    this.failure = null;
    this.connection = "connecting";
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`);
    this.socket = socket;
    socket.onopen = () => {
      this.delay = 400;
      this.connection = "connected";
      this.publish();
    };
    socket.onmessage = (message) => {
      const frame = JSON.parse(String(message.data));
      if (typeof frame.event === "string") {
        for (const listener of this.eventListeners) listener(frame.event, frame.data);
        return;
      }
      const waiting = this.pending.get(frame.id);
      if (!waiting) return;
      this.pending.delete(frame.id);
      if (frame.error) waiting.reject(new Error(frame.error.message ?? "Request failed"));
      else waiting.resolve(frame.result ?? null);
    };
    socket.onclose = () => {
      if (this.socket !== socket) return;
      this.socket = null;
      for (const waiting of this.pending.values()) waiting.reject(new Error("The CLI connection dropped"));
      this.pending.clear();
      const was = this.connection;
      this.connection = "disconnected";
      if (was !== "disconnected") this.publish();
      const delay = this.delay;
      this.delay = Math.min(this.delay * 1.6, 5000);
      setTimeout(() => this.open(), delay);
    };
  }

  async state() {
    return this.current();
  }

  request(method: string, params: Record<string, unknown> = {}) {
    const socket = this.socket;
    if (!socket || this.connection !== "connected") return Promise.reject(new Error("The Beans CLI is not running"));
    const id = this.nextID++;
    return new Promise<unknown>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params }));
    });
  }

  reconnect() {
    if (this.failure && !this.socket) this.open();
    else this.socket?.close();
  }

  onEvent(listener: (name: string, data: unknown) => void) {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  }

  onState(listener: (state: CLIState) => void) {
    this.stateListeners.add(listener);
    return () => this.stateListeners.delete(listener);
  }
}

let transport: Transport | null = null;

export function cliTransport(): Transport {
  transport ??= inApp ? new AppTransport() : new SocketTransport();
  return transport;
}

// MARK: - Windows

export function onMenuCommand(listener: (id: string) => void): () => void {
  return inApp ? events.menuCommand.on(listener) : () => {};
}

export function onOpenChat(listener: (chatID: string) => void): () => void {
  return inApp ? events.openChat.on(listener) : () => {};
}

export function onUpdaterChanged(listener: (state: UpdaterState) => void): () => void {
  return inApp ? events.updaterChanged.on(listener) : () => {};
}

// The native updater asks only for its own dialog words through the live L()
// table, rather than maintaining a separate native translation dictionary.
if (typeof window !== "undefined") {
  Object.defineProperty(window, "beansUpdateWord", {
    configurable: true,
    value: ({ key, args }: { key: string; args?: string[] }): string | null => {
      switch (key) {
        case "The Beans update is not ready":
          return L("The Beans update is not ready");
        case "You're up to date":
          return L("You're up to date");
        case "Beans %@ is available":
          return L("Beans %@ is available", args?.[0] ?? "");
        case "Install on normal quit after bot work finishes and drafts are saved. The app will not restart automatically.":
          return L("Install on normal quit after bot work finishes and drafts are saved. The app will not restart automatically.");
        case "Install on Quit":
          return L("Install on Quit");
        case "Later":
          return L("Later");
        case "Skip This Version":
          return L("Skip This Version");
        case "Download and install updates automatically":
          return L("Download and install updates automatically");
        case "The update will wait":
          return L("The update will wait");
        default:
          return null;
      }
    },
  });
}

/** Whether this page's window is where the user looks: in front, shown, not minimized. */
export function watchWindowState(listener: (state: WindowState) => void): () => void {
  if (inApp) {
    let disposed = false;
    void Host.windowState().then((state) => {
      if (!disposed) listener(state);
    }, (error: unknown) => {
      console.error("Host.WindowState failed:", error);
    });
    const unsubscribe = events.windowState.on(listener);
    return () => {
      disposed = true;
      unsubscribe();
    };
  }
  const report = () => listener({ focused: document.hasFocus(), visible: document.visibilityState === "visible", minimized: false, fullScreen: false });
  report();
  window.addEventListener("focus", report);
  window.addEventListener("blur", report);
  document.addEventListener("visibilitychange", report);
  return () => {
    window.removeEventListener("focus", report);
    window.removeEventListener("blur", report);
    document.removeEventListener("visibilitychange", report);
  };
}

export const host = {
  showMainWindow: () => (inApp ? Host.showMainWindow() : Promise.resolve()),
  // A browser tab has one window: the page goes to the other route, with its query (`?mock=1`).
  finishOnboarding: () => (inApp ? Host.finishOnboarding() : Promise.resolve(location.assign(`/${location.search}`))),
  showOnboarding: () => (inApp ? Host.showOnboarding() : Promise.resolve(location.assign(`/onboarding${location.search}`))),
  toggleFullScreen: () => (inApp ? Host.toggleFullScreen() : document.fullscreenElement ? document.exitFullscreen() : document.documentElement.requestFullscreen()),
  windowRoom: () => (inApp ? Host.windowRoom(window.innerWidth) : Promise.resolve(0)),
  widenWindow: (by: number) => (inApp ? Host.widenWindow(by, window.innerWidth) : Promise.resolve()),
  setBadge: (count: number) => (inApp ? Host.setBadge(count) : Promise.resolve()),
  setTrayMenu: (open: string, quit: string) => (inApp ? Host.setTrayMenu(open, quit) : Promise.resolve()),
  openExternal: (url: string) => (inApp ? Host.openExternal(url) : Promise.resolve(void window.open(url, "_blank"))),
  copyText: (text: string) => (inApp ? Host.copyText(text) : navigator.clipboard.writeText(text)),
  beep: () => (inApp ? Host.beep() : Promise.resolve()),
  updaterState: (): Promise<UpdaterState> =>
    inApp
      ? Host.updaterState()
      : Promise.resolve({ version: "dev", lastCheck: 0, automaticChecks: false, automaticDownloads: false }),
  /** Opens MyGo's update window, which says what the check finds. */
  checkForUpdates: () => (inApp ? Host.checkForUpdates() : Promise.resolve()),
  setAutomaticUpdates: (checks: boolean, downloads: boolean) => Host.setAutomaticUpdates(checks, downloads),
};

// MARK: - Files

export const files = {
  choose: (options: ChooseOptions): Promise<FileInfo[]> => (inApp ? Files.choose(options) : Promise.resolve([])),
  inspect: (paths: string[]): Promise<FileInfo[]> => (inApp ? Files.inspect(paths) : Promise.resolve([])),
  url: (path: string): Promise<string> => (inApp ? Files.url(path) : Promise.resolve("")),
  open: (path: string) => (inApp ? Files.open(path) : Promise.resolve()),
  showInFolder: (path: string) => (inApp ? Files.showInFolder(path) : Promise.resolve()),
  /** A bot's profile image made from a picture: square, upright, at most 512 px. */
  prepareAvatar: (path: string): Promise<FileInfo> => Files.prepareAvatar(path),
  /** Writes pasted bytes to a temporary file the CLI can read. */
  async savePasted(name: string, blob: Blob): Promise<FileInfo> {
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = "";
    for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return Files.savePasted(name, btoa(binary));
  },
};

export function onDroppedFiles(listener: (drop: FileDrop) => void): () => void {
  return inApp ? onFileDrop(listener) : () => {};
}

// MARK: - Menus and notifications

export const menus = {
  setBar: (items: MenuItemSpec[]) => (inApp ? Menus.setBar(items) : Promise.resolve()),
  update: (states: MenuItemState[]) => (inApp ? Menus.update(states) : Promise.resolve()),
  popup: (items: MenuItemSpec[], x: number, y: number) => Menus.popup(items, x, y),
};

export const notices = {
  supported: () => (inApp ? Notices.supported() : Promise.resolve(false)),
  show: (options: NoticeOptions) => (inApp ? Notices.show(options) : Promise.resolve()),
  remove: (id: string) => (inApp ? Notices.remove(id) : Promise.resolve()),
  clearChat: (chatID: string) => (inApp ? Notices.clearChat(chatID) : Promise.resolve()),
};

export function cliReconnect(): void {
  cliTransport().reconnect();
}
