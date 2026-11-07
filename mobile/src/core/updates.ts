// GitHub metadata discovers candidates; only the native signed-readiness verifier can offer one.
import { create } from "zustand";
import { Alert, AppState } from "react-native";
import { androidUpdates } from "../../modules/beans-core";
import { t } from "../i18n";
import { loadPrefs, savePrefs } from "./prefs";
import { UpdateController, type UpdateOffer } from "./updateController";
import { updateCheckDue, updateIsSkipped } from "./updatePolicy";
import { hasUpdateDrafts, subscribeComposerDrafts } from "./updateDrafts";
export { updateCandidates, updateCheckDue, updateIsSkipped, type UpdatePreferences } from "./updatePolicy";

export type UpdateBlock = "store_managed" | "development" | "native_verifier_unavailable";
export type UpdateStatus = "blocked" | "idle" | "checking" | "offered" | "downloading" | "installing" | "confirmation" | "error";

export function updateBlock(platform: string, applicationId: string | null): UpdateBlock | null {
  if (applicationId === "ai.amoena.beans.dev") return "development";
  if (platform !== "android" || applicationId !== "ai.amoena.beans") return "store_managed";
  const capability = androidUpdates.capability();
  return capability === "supported" ? null : capability;
}

export function downloadProgress(downloaded: number, total: number): number {
  if (!Number.isFinite(downloaded) || !Number.isFinite(total) || total <= 0) return 0;
  return Math.max(0, Math.min(1, downloaded / total));
}

export function isUpdateProgressActive(status: UpdateStatus): boolean {
  return status === "downloading" || status === "installing" || status === "confirmation";
}

const controller = new UpdateController(androidUpdates);
let generation = 0;
let activeInstall: string | null = null;
let warnedDraftInstall: string | null = null;
let resultTimer: ReturnType<typeof setInterval> | undefined;
export const useUpdates = create<{
  status: UpdateStatus;
  reason: UpdateBlock | null;
  offer: UpdateOffer | null;
  installingVersion: string | null;
  installingId: string | null;
  error: string | null;
  checks: boolean;
  progress: number;
}>()(() => ({ status: "blocked", reason: "native_verifier_unavailable", offer: null, installingVersion: null, installingId: null, error: null, checks: true, progress: 0 }));

function stopResultChecks(): void {
  if (resultTimer !== undefined) clearInterval(resultTimer);
  resultTimer = undefined;
}

function watchInstallResult(): void {
  if (resultTimer === undefined) resultTimer = setInterval(readInstallResult, 1000);
}

function installFailed(error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  activeInstall = null;
  stopResultChecks();
  useUpdates.setState({ status: "error", offer: controller.offer, installingVersion: null, installingId: null, error: message, progress: 0 });
  Alert.alert(t("Couldn’t install the update"), message);
}

function readInstallResult(): boolean {
  if (AppState.currentState !== "active") return false;
  const result = androidUpdates.result();
  if (!result) return false;
  generation++;
  activeInstall = null;
  stopResultChecks();
  if (result !== "installed") installFailed(result);
  else useUpdates.setState({ status: "idle", offer: null, installingVersion: null, installingId: null, error: null, progress: 0 });
  return true;
}

// Both the daily alert and Settings capture the exact native offer id in their button closure.
export function offerUpdate(offer: UpdateOffer): void {
  if (useUpdates.getState().reason || AppState.currentState !== "active" || controller.offer?.id !== offer.id) return;
  Alert.alert(t("Beans {version} is available", { version: offer.version }), `${offer.notes}\n\n${t("Save or send all drafts first. Installing replaces this process. Have you saved your changes?")}`, [
    { text: t("Skip This Version"), onPress: () => { if (controller.offer?.id === offer.id) dismissUpdate(true); } },
    { text: t("Remind Me Later"), style: "cancel", onPress: () => { if (controller.offer?.id === offer.id) dismissUpdate(); } },
    { text: t("Install Update"), onPress: () => void installUpdate(offer.id, true) },
  ], { cancelable: false });
}

export async function checkUpdates(manual = true): Promise<void> {
  const state = useUpdates.getState();
  if (state.reason || controller.busy || isUpdateProgressActive(state.status) || AppState.currentState !== "active") return;
  const prefs = loadPrefs();
  if (!manual && !updateCheckDue(prefs, Date.now())) return;
  const current = ++generation;
  useUpdates.setState({ status: "checking", offer: null, error: null });
  savePrefs({ ...prefs, update_checked_at: Date.now() });
  try {
    await controller.check();
    if (generation !== current) return;
    if (controller.offer && updateIsSkipped(loadPrefs(), controller.offer.version, manual)) controller.dismiss();
    useUpdates.setState({ status: controller.offer ? "offered" : "idle", offer: controller.offer });
    if (!manual && controller.offer) offerUpdate(controller.offer);
  } catch (error) {
    if (generation === current) useUpdates.setState({ status: "error", error: error instanceof Error ? error.message : String(error) });
  }
}

export function dismissUpdate(skip = false): boolean {
  // System confirmation owns a committed session; hiding the dialog never cancels it.
  if (useUpdates.getState().status === "installing" || useUpdates.getState().status === "confirmation") return false;
  const offered = controller.offer;
  if (!controller.dismiss()) {
    useUpdates.setState({ status: "confirmation", offer: null, progress: 1 });
    watchInstallResult();
    return false;
  }
  generation++;
  activeInstall = null;
  stopResultChecks();
  if (skip && offered) savePrefs({ ...loadPrefs(), update_skipped_version: offered.version });
  useUpdates.setState({ status: "idle", offer: null, installingVersion: null, installingId: null, error: null, progress: 0 });
  return true;
}

export function setUpdateChecks(checks: boolean): void {
  savePrefs({ ...loadPrefs(), update_checks: checks });
  useUpdates.setState({ checks });
}

export async function installUpdate(id: string, draftsSaved: boolean): Promise<void> {
  if (AppState.currentState !== "active" || useUpdates.getState().reason || controller.busy || activeInstall) return;
  if (controller.offer?.id !== id) return;
  const current = ++generation;
  const version = controller.offer.version;
  try {
    // Check before opening the progress dialog; the controller checks again at consumption.
    if (!draftsSaved || hasUpdateDrafts()) throw new Error("Save or send drafts before installing");
    activeInstall = id;
    warnedDraftInstall = null;
    useUpdates.setState({ status: "downloading", offer: null, installingVersion: version, installingId: id, error: null, progress: 0 });
    await controller.install(id, hasUpdateDrafts());
    if (generation !== current) return;
    useUpdates.setState({ status: "confirmation" });
    if (!readInstallResult()) watchInstallResult();
  } catch (error) {
    if (generation === current) installFailed(error);
  }
}

export function initializeUpdates(platform: string, applicationId: string | null): () => void {
  generation++;
  activeInstall = null;
  stopResultChecks();
  const reason = updateBlock(platform, applicationId);
  useUpdates.setState({ status: reason ? "blocked" : "idle", reason, offer: null, installingVersion: null, installingId: null, error: null, progress: 0, checks: loadPrefs().update_checks !== false });
  if (reason) return () => {};
  const removeProgress = androidUpdates.onProgress(({ id, phase, downloaded, total }) => {
    const state = useUpdates.getState();
    if (id !== activeInstall || !isUpdateProgressActive(state.status)) return;
    if (phase === "installing" && state.status === "downloading") useUpdates.setState({ status: "installing", progress: 1 });
    else if (phase === "downloading" && state.status === "downloading") {
      useUpdates.setState({ progress: Math.max(state.progress, downloadProgress(downloaded, total)) });
    }
  });
  const removeDrafts = subscribeComposerDrafts(() => {
    if (isUpdateProgressActive(useUpdates.getState().status) && hasUpdateDrafts()) {
      if (dismissUpdate()) installFailed(new Error("Save or send drafts before installing"));
      else if (warnedDraftInstall !== activeInstall) {
        warnedDraftInstall = activeInstall;
        Alert.alert(t("Install Update"), t("Android is already confirming this installation. Cancel it to keep new drafts."));
      }
    }
  });
  if (!readInstallResult()) void checkUpdates(false);
  const subscription = AppState.addEventListener("change", state => {
    if (state === "active") {
      if (!readInstallResult()) void checkUpdates(false);
    } else dismissUpdate();
  });
  return () => {
    generation++;
    activeInstall = null;
    stopResultChecks();
    subscription.remove();
    removeProgress();
    removeDrafts();
    controller.dismiss();
  };
}
