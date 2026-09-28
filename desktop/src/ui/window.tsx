// What every window of the app sets up, after the macOS app's AppDelegate actions: its menu bar and
// the commands any window answers (Help, Architecture Notes, About, Check for Updates, and the Debug
// menu's), and the update offer.

import { host, hostInfo, inApp, preferences, setPreferences, type UpdateInfo } from "../host";
import { L } from "../l10n";
import { errorText, store } from "../model/store";
import { commands, installMenuBar, setCheckingForUpdates } from "./commands";
import { popupMenu } from "./menu";
import { alert } from "./overlay";

function presentNote(title: string, body: string): void {
  void alert({ message: title, informative: body, style: "informational", buttons: [{ title: L("OK") }], width: 420 });
}

export function showHelp(): void {
  presentNote(
    L("Lorca runs on Devices you own"),
    L(
      "Every bot is assigned to a Runner: a Device running macOS, Linux, or Windows. That machine's CLI runs the turn with your account's provider credentials, so a bot on an offline Runner waits until it reconnects. Phones and tablets pair as Devices but never run bots.\n\nThe app talks only to the local CLI on 127.0.0.1:%@. Start it with `lorca serve`; the CLI holds your keys and provider credentials, which reach your other Devices encrypted.",
      String(preferences().cliPort),
    ).replace("lorca serve", hostInfo().cliCommand),
  );
}

export function showArchitecture(): void {
  presentNote(
    L("Three processes"),
    L(
      "The app talks only to the local CLI over a localhost websocket. The CLI holds the keys, runs the agent loop, and syncs ciphertext with the relay. The relay stores public keys and opaque blobs.\n\nFull notes live in ARCHITECTURE.md.",
    ),
  );
}

export function showAbout(): void {
  void alert({ message: hostInfo().name, informative: L("Version %@", hostInfo().version), buttons: [{ title: L("OK") }] });
}

/** Offers a newer version: install it now, which relaunches the app, later, or never, as Sparkle's
 * alert does. An automatic check passes over a version the user skipped. */
export async function offerUpdate(update: UpdateInfo, options: { automatic?: boolean } = {}): Promise<void> {
  if (options.automatic && preferences().skippedUpdate === update.version) return;
  const name = hostInfo().name;
  const answer = await alert({
    message: L("A new version of %@ is available!", name),
    informative: [L("%@ %@ is now available—you have %@. Would you like to install it now?", name, update.version, hostInfo().version), update.notes.trim()]
      .filter((part) => part !== "")
      .join("\n\n"),
    buttons: [{ title: L("Install Update") }, { title: L("Remind Me Later") }, { title: L("Skip This Version") }],
    escape: 1,
    width: 440,
  });
  if (answer === 2) void setPreferences({ skippedUpdate: update.version });
  if (answer !== 0) return;
  try {
    await host.installUpdate();
  } catch (error) {
    void alert({ message: L("Update Error!"), informative: errorText(error) });
  }
}

/** A check the user asked for, from the menu or Settings: its answer, whatever it is, shows. */
let checking = false;
export async function checkForUpdates(): Promise<void> {
  if (checking) return;
  checking = true;
  setCheckingForUpdates(true);
  try {
    const found = await host.checkForUpdates();
    if (found) await offerUpdate(found);
    else void alert({ message: L("You’re up to date!"), informative: L("%@ %@ is currently the newest version available.", hostInfo().name, hostInfo().version) });
  } catch (error) {
    void alert({ message: L("Update Error!"), informative: errorText(error) });
  } finally {
    checking = false;
    setCheckingForUpdates(false);
  }
}

/** A right-click gets the app's menus, never the webview's (Back, Reload, Print): over selected text
 * the Copy a text view offers, and in a field the system's own menu for editing. A control with a
 * menu of its own has shown it by now. */
function installContextMenu(): () => void {
  if (!inApp) return () => {};
  const onMenu = (event: MouseEvent) => {
    if (event.defaultPrevented) return;
    if ((event.target as Element).closest("input, textarea, [contenteditable]")) return;
    event.preventDefault();
    const text = window.getSelection()?.toString() ?? "";
    if (text.trim() === "") return;
    void popupMenu([{ id: "copy", label: L("Copy") }], { x: event.clientX, y: event.clientY }).then((picked) => {
      if (picked === "copy") void host.copyText(text);
    });
  };
  window.addEventListener("contextmenu", onMenu);
  return () => window.removeEventListener("contextmenu", onMenu);
}

/** The menu bar and the commands any window answers. The main window adds its own. */
export function setupWindow(kind: "main" | "other"): () => void {
  commands.register({
    help: showHelp,
    architecture: showArchitecture,
    about: showAbout,
    checkForUpdates: () => void checkForUpdates(),
    simulateOffline: () => (store.isMock ? store.setConnected(!store.isConnected) : store.reconnect()),
    replayMock: () => store.resetMockData(),
    showOnboarding: () => void host.showOnboarding(),
  });
  // The tray menu speaks the app's language from the first window on, onboarding's included.
  void host.setTrayMenu(L("Open %@", hostInfo().name), L("Quit %@", hostInfo().name));
  const offMenu = installContextMenu();
  const offBar = installMenuBar(kind);
  return () => {
    offMenu();
    offBar();
  };
}
