// App-global update policy. GitHub release metadata identifies candidates, never install authority.
import { create } from "zustand";
import { AppState } from "react-native";
import { androidUpdates } from "../../modules/beans-core";
import { loadPrefs, savePrefs } from "./prefs";
import { UpdateController, type UpdateOffer } from "./updateController";
export { updateCandidates, updateCheckDue, updateIsSkipped, type UpdatePreferences } from "./updatePolicy";
import { updateCheckDue, updateIsSkipped } from "./updatePolicy";
import { hasUpdateDrafts } from "./updateDrafts";


export type UpdateBlock = "store_managed" | "development" | "native_verifier_unavailable";

export function updateBlock(platform: string, applicationId: string | null): UpdateBlock | null {
  if (applicationId === "ai.amoena.beans.dev") return "development";
  if (platform !== "android") return "store_managed";
  const capability = androidUpdates.capability();
  return capability === "supported" ? null : capability;
}

const controller = new UpdateController(androidUpdates);
export const useUpdates = create<{
  status: "blocked" | "idle" | "checking" | "offered" | "installing" | "confirmation" | "error";
  reason: UpdateBlock | null; offer: UpdateOffer | null; error: string | null; checks: boolean; progress: number;
}>()(() => ({ status: "blocked", reason: "native_verifier_unavailable", offer: null, error: null, checks: true, progress: 0 }));

export async function checkUpdates(manual = true): Promise<void> {
  if (useUpdates.getState().reason || controller.busy || AppState.currentState !== "active") return;
  const prefs = loadPrefs();
  if (!manual && !updateCheckDue(prefs, Date.now())) return;
  useUpdates.setState({ status: "checking", offer: null, error: null });
  savePrefs({ ...prefs, update_checked_at: Date.now() });
  try {
    await controller.check();
    if (controller.offer && updateIsSkipped(loadPrefs(), controller.offer.version, manual)) controller.dismiss();
    useUpdates.setState({ status: controller.offer ? "offered" : "idle", offer: controller.offer });
  } catch (error) { useUpdates.setState({ status: "error", error: error instanceof Error ? error.message : String(error) }); }
}
export function dismissUpdate(skip = false): void {
  if (skip && controller.offer) savePrefs({ ...loadPrefs(), update_skipped_version: controller.offer.version });
  controller.dismiss(); useUpdates.setState({ status: "idle", offer: null });
}
export function setUpdateChecks(checks: boolean): void {
  savePrefs({ ...loadPrefs(), update_checks: checks }); useUpdates.setState({ checks });
}
export async function installUpdate(id: string, draftsSaved: boolean): Promise<void> {
  if (AppState.currentState !== "active" || useUpdates.getState().reason) return;
  useUpdates.setState({ status: "installing", error: null });
  try {
    await controller.install(id, !draftsSaved || hasUpdateDrafts());
    useUpdates.setState({ status: "confirmation", offer: null });
  } catch (error) { useUpdates.setState({ status: "error", offer: controller.offer, error: error instanceof Error ? error.message : String(error) }); }
}
export function initializeUpdates(platform: string, applicationId: string | null): () => void {
  const reason = updateBlock(platform, applicationId);
  useUpdates.setState({ status: reason ? "blocked" : "idle", reason, checks: loadPrefs().update_checks !== false });
  if (reason) return () => {};
  const removeProgress = androidUpdates.onProgress(({ downloaded, total }) => {
    useUpdates.setState({ progress: total > 0 ? Math.floor(downloaded * 100 / total) : 0 });
  });
  void checkUpdates(false);
  const subscription = AppState.addEventListener("change", state => {
    if (state === "active") {
      const result = androidUpdates.result();
      if (result && result !== "installed") useUpdates.setState({ status: "error", error: result });
      else void checkUpdates(false);
    }
    else dismissUpdate();
  });
  return () => { subscription.remove(); removeProgress(); controller.dismiss(); };
}
