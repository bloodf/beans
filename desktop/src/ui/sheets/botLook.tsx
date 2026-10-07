// A whole generated appearance and optional photo are edited as one local draft.
// Shared numeric geometry renders the selected base/state without replacing SVG nodes.

import { createEffect, createMemo, createSignal, onSettled, Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import { resolveBotAppearance, botAppearanceContrast } from "@beans/blobatar";
import { files, type FileInfo } from "../../host";
import { L } from "../../l10n";
import {
  avatarStates, avatarShapes, avatarExpressions, avatarBackgrounds, avatarTones,
  editAppearance, resetAppearance, shuffleAppearance, lookProblem, lookErrors, previewAppearance,
  type AppearanceSelection, type BotAppearance, type BotLook, type LookPreview,
} from "../../model/botLook";
import { errorText, store } from "../../model/store";
import { Avatar, botAvatar, type AvatarContent } from "../avatar";
import { Button, PopUpButton, TextField } from "../controls";
import { avatarClock } from "../avatarClock";
import { watchAvatarVisibility } from "../avatarVisibility";
import { alert, presentSheet, Sheet } from "../overlay";

/** Longest side of a stored profile image, as `Files.PrepareAvatar` makes it. */
const imageSide = 512;

type ImageChange = { kind: "keep" } | { kind: "remove" } | { kind: "set"; file: FileInfo };

export function presentBotLook(botID: string): void {
  presentSheet((dismiss) => <BotLookSheet botID={botID} dismiss={dismiss} />);
}

function LookField(props: { label: string; children: JSX.Element }) {
  return <div class="look-field"><span class="look-field-label">{props.label}</span>{props.children}</div>;
}

function BotLookSheet(props: { botID: string; dismiss: () => void }) {
  const [imageChange, setImageChange] = createSignal<ImageChange>({ kind: "keep" });
  const [look, setLook] = createSignal<BotLook | null | undefined>(store.bot(props.botID)?.look);
  const [lookChanged, setLookChanged] = createSignal(false);
  const [selection, setSelection] = createSignal<AppearanceSelection>("base");
  const [cycling, setCycling] = createSignal(false);
  const [cycleState, setCycleState] = createSignal<typeof avatarStates[number]>("idle");
  const [previewReducedMotion, setPreviewReducedMotion] = createSignal(false);
  let previewElement: HTMLDivElement | undefined;
  let previewVisible = false, cycleStarted = 0, cycleIndex = 0;
  let stopCycle: (() => void) | undefined;
  // Keep partially typed numbers intact, including across state switches.
  const [hueTexts, setHueTexts] = createSignal<Partial<Record<AppearanceSelection, string>>>({});
  const [choosingImage, setChoosingImage] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [saveError, setSaveError] = createSignal<string>();
  const busy = () => saving() || choosingImage();
  const selected = () => {
    const state = selection();
    return state === "base" ? look()?.base ?? {} : look()?.states?.[state] ?? {};
  };
  const [validPreview, setValidPreview] = createSignal<LookPreview>({ look: look(), state: "idle" });
  createEffect(() => ({ look: look(), selection: cycling() ? cycleState() : selection() }), (draft) => {
    setValidPreview(previewAppearance(draft.look, draft.selection, validPreview()));
  });
  const problem = () => lookProblem(look());
  const contrast = createMemo(() => {
    if (problem()) return undefined;
    const preview = validPreview();
    const resolved = resolveBotAppearance(props.botID, preview.look, preview.state);
    return { ...botAppearanceContrast(resolved), background: resolved.background };
  });
  const incompatible = () => problem() === lookErrors.unsupported;
  const synchronizeCycle = () => {
    if (cycling() && previewVisible && !previewReducedMotion() && !problem()) {
      if (!stopCycle) {
        cycleStarted = performance.now();
        stopCycle = avatarClock.subscribe((time) => {
          if (time - cycleStarted < 1800) return;
          cycleStarted = time;
          cycleIndex = (cycleIndex + 1) % avatarStates.length;
          setCycleState(avatarStates[cycleIndex]!);
        });
      }
    } else {
      stopCycle?.(); stopCycle = undefined;
    }
  };
  createEffect(() => ({ cycling: cycling(), invalid: problem() }), synchronizeCycle);
  onSettled(() => {
    const unwatch = previewElement ? watchAvatarVisibility(previewElement, (visible, reducedMotion) => {
      previewVisible = visible;
      setPreviewReducedMotion(reducedMotion);
      if (reducedMotion) setCycling(false);
      synchronizeCycle();
    }) : undefined;
    return () => { stopCycle?.(); unwatch?.(); };
  });
  const edit = (patch: BotAppearance) => {
    setLook(editAppearance(look(), selection(), patch));
    setLookChanged(true);
    setSaveError(undefined);
  };
  const clearHueText = () => {
    const texts = { ...hueTexts() };
    delete texts[selection()];
    setHueTexts(texts);
  };
  const labels: Record<string, string> = {
    round: L("Round"), organic: L("Organic"), boxy: L("Boxy"), capsule: L("Capsule"),
    nub: L("Nub"), cloud: L("Cloud"), droplet: L("Droplet"), hexagon: L("Hexagon"), sun: L("Sun"), triangle: L("Triangle"),
    idle: L("Idle"), happy: L("Happy"), sad: L("Sad"), mad: L("Mad"), surprised: L("Surprised"), wink: L("Wink"),
    sleepy: L("Sleepy"), smug: L("Smug"), unsure: L("Unsure"), scared: L("Scared"), love: L("Love"), shy: L("Shy"), sick: L("Sick"), thinking: L("Thinking"),
    none: L("None"), square: L("Square"), circle: L("Circle"), squircle: L("Squircle"),
    pastel: L("Pastel"), pale: L("Pale"), mid: L("Mid"), deep: L("Deep"), bright: L("Bright"), ink: L("Ink"),
    responding: L("Responding"), working: L("Working"), waiting: L("Waiting"), retry: L("Retry"), error: L("Error"),
  };
  const options = <T extends string>(values: readonly T[], baseLabel: string) => {
    const items: { value: T | undefined; label: string }[] = [
      { value: undefined, label: selection() === "base" ? baseLabel : L("Inherit base") },
    ];
    return [...items, ...values.map((value) => ({ value, label: labels[value]! }))];
  };
  const reset = () => {
    setLook(resetAppearance(look(), selection()));
    if (selection() === "base") setHueTexts({});
    else clearHueText();
    setLookChanged(true);
    setSaveError(undefined);
  };
  const shuffle = () => {
    setLook(shuffleAppearance(look(), selection()));
    clearHueText();
    setLookChanged(true);
    setSaveError(undefined);
  };

  /** Whether the saved look, with the pending change applied, has an image. */
  const hasImage = () => {
    const change = imageChange();
    if (change.kind === "keep") return store.bot(props.botID)?.avatar !== undefined;
    return change.kind === "set";
  };

  const preview = (): AvatarContent => {
    const change = imageChange();
    if (change.kind === "set") return { kind: "image", url: change.file.url };
    if (change.kind === "keep") {
      const bot = store.bot(props.botID);
      const saved = bot ? botAvatar(bot) : undefined;
      if (saved?.kind === "image") return saved;
    }
    return { kind: "bot", id: props.botID, look: store.bot(props.botID)?.look, state: "idle" };
  };

  const chooseImage = async () => {
    if (busy()) return;
    setChoosingImage(true);
    try {
      const [picked] = await files.choose({ images: true, message: L("Choose an image for this bot.") });
      if (picked) setImageChange({ kind: "set", file: await files.prepareAvatar(picked.path) });
    } catch {
      void alert({ message: L("That file could not be read as an image.") });
    } finally {
      setChoosingImage(false);
    }
  };

  const save = async () => {
    if (busy() || problem()) return;
    setSaving(true);
    setSaveError(undefined);
    try {
      const change = imageChange();
      if (lookChanged() || change.kind !== "keep") {
        await store.saveBotLook(props.botID, lookChanged() ? look() : undefined, change.kind === "keep" ? undefined : change.kind === "set" ? change.file : null);
      }
      props.dismiss();
    } catch (error) {
      setSaveError(errorText(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Sheet
      title={L("Look")}
      subtitle={L("Customize the base appearance and its activity states. Photos take priority and stay independent.")}
      width={520}
      confirm={L("Save")}
      confirmDisabled={busy() || !!problem()}
      onConfirm={() => void save()}
      onCancel={() => { if (!busy()) props.dismiss(); }}
      class="look-sheet"
    >
      <div class="look-preview" ref={(element) => (previewElement = element)}>
        <Avatar content={{ kind: "bot", id: props.botID, look: validPreview().look ?? undefined, state: validPreview().state }} size={72} />
        <Show when={problem()}><div class="look-caption">{L("Complete valid fields to update the preview.")}</div></Show>
        <Show when={hasImage()}><Avatar content={preview()} size={48} /></Show>
        <div>
          <div>{L("Generated preview")} · {labels[validPreview().state]}</div>
          <div class="look-caption">{hasImage() ? L("Your photo takes priority. Editing generated appearance does not remove it.") : L("Preview follows the selected appearance state.")}</div>
          <Button small disabled={busy() || !!problem() || previewReducedMotion()} onClick={() => {
            cycleIndex = 0; setCycleState("idle"); setCycling(!cycling());
          }}>{cycling() ? L("Stop preview") : L("Cycle activity states")}</Button>
        </div>
      </div>
      <fieldset class="look-fields" disabled={busy() || incompatible()}>
        <legend>{L("Generated appearance")}</legend>
        <LookField label={L("Appearance")}>
          <PopUpButton<AppearanceSelection> label={L("Appearance")} value={selection()} onChange={(state) => { setCycling(false); setSelection(state); }}
            options={[{ value: "base", label: L("Base appearance") }, ...avatarStates.map((value) => ({ value, label: labels[value]! }))]} />
        </LookField>
        <div class="look-caption look-inheritance">
          {selection() === "base" ? L("Unset values keep the bot's seeded look. The default expression is Idle, background is None, and motion is On.") : L("Unset values inherit the base appearance, including individual colors and motion.")}
        </div>
        <LookField label={L("Shape")}>
          <PopUpButton label={L("Shape")} value={selected().shape} options={options(avatarShapes, L("Seeded default"))} onChange={(shape) => edit({ shape })} />
        </LookField>
        <LookField label={L("Expression")}>
          <PopUpButton label={L("Expression")} value={selected().expression} options={options(avatarExpressions, L("Default (%@)", L("Idle")))} onChange={(expression) => edit({ expression })} />
        </LookField>
        <LookField label={L("Background")}>
          <PopUpButton label={L("Background")} value={selected().background} options={options(avatarBackgrounds, L("Default (%@)", L("None")))} onChange={(background) => edit({ background })} />
        </LookField>
        <LookField label={L("Hue")}>
          <TextField label={L("Hue")} spellcheck={false} value={hueTexts()[selection()] ?? selected().hue?.toString() ?? ""}
            placeholder={selection() === "base" ? L("Seeded default") : L("Inherit base")}
            onInput={(text) => {
              setHueTexts({ ...hueTexts(), [selection()]: text });
              edit({ hue: text.trim() === "" ? undefined : Number(text) });
            }} />
        </LookField>
        <LookField label={L("Tone")}>
          <PopUpButton label={L("Tone")} value={selected().tone} options={options(avatarTones, L("Seeded default"))} onChange={(tone) => edit({ tone })} />
        </LookField>
        <LookField label={L("Body color")}>
          <TextField label={L("Body color")} spellcheck={false} monospaced value={selected().palette?.head ?? ""}
            placeholder={selection() === "base" ? L("Automatic") : look()?.base.palette?.head ?? L("Inherit base")}
            onInput={(text) => edit({ palette: { head: text.trim() === "" ? undefined : text.toUpperCase() } })} />
        </LookField>
        <LookField label={L("Eye color")}>
          <TextField label={L("Eye color")} spellcheck={false} monospaced value={selected().palette?.eye ?? ""}
            placeholder={selection() === "base" ? L("Automatic") : look()?.base.palette?.eye ?? L("Inherit base")}
            onInput={(text) => edit({ palette: { eye: text.trim() === "" ? undefined : text.toUpperCase() } })} />
        </LookField>
        <LookField label={L("Background color")}>
          <TextField label={L("Background color")} spellcheck={false} monospaced value={selected().palette?.bg ?? ""}
            placeholder={selection() === "base" ? L("Automatic") : look()?.base.palette?.bg ?? L("Inherit base")}
            onInput={(text) => edit({ palette: { bg: text.trim() === "" ? undefined : text.toUpperCase() } })} />
        </LookField>
        <LookField label={L("Motion")}>
          <PopUpButton label={L("Motion")} value={selected().motion === undefined ? "" : selected().motion ? "on" : "off"}
            options={[{ value: "", label: selection() === "base" ? L("Default (%@)", L("On")) : L("Inherit base") }, { value: "on", label: L("On") }, { value: "off", label: L("Off") }]}
            onChange={(motion) => edit({ motion: motion === "" ? undefined : motion === "on" })} />
        </LookField>
        <div class="look-caption">{L("Colors use #RRGGBB. Leave a color or hue empty to inherit. Hue is 0 to less than 360. Reduce Motion always wins.")}</div>
        <Show when={(contrast()?.eyeOnHead ?? 3) < 3}><div class="look-warning" role="status">{L("Body and eye colors have low contrast. Your colors will still be saved.")}</div></Show>
        <Show when={contrast()?.background !== "none" && (contrast()?.headOnBg ?? 1.5) < 1.5}><div class="look-warning" role="status">{L("Body and background colors have low contrast. Your colors will still be saved.")}</div></Show>
        <div class="look-actions">
          <Button onClick={shuffle}>{L("Shuffle")}</Button>
          <Button onClick={reset}>{selection() === "base" ? L("Restore all defaults") : L("Reset state")}</Button>
        </div>
        <div class="look-caption">{selection() === "base" ? L("Restoring seeded defaults clears all generated appearance overrides, not the photo.") : L("Reset state removes this state's overrides and inherits the base appearance.")}</div>
      </fieldset>
      <section aria-label={L("Image")}>
        <h3 class="look-heading">{L("Image")}</h3>
        <div class="look-image-buttons">
          <Button disabled={busy() || incompatible()} onClick={() => void chooseImage()}>{L("Choose Image…")}</Button>
          <Show when={hasImage()}>
            <Button disabled={busy() || incompatible()} onClick={() => setImageChange({ kind: "remove" })}>{L("Remove Image")}</Button>
          </Show>
        </div>
        <Show when={hasImage()}><div class="look-caption">{L("Your photo takes priority. Editing generated appearance does not remove it.")}</div></Show>
        <div class="look-caption">{L("Images are resized to %d px and shared encrypted, like an attachment.", imageSide)}</div>
      </section>
      <Show when={problem()}><div class="look-save-error" role="alert">{incompatible() ? L("Unsupported avatar appearance. Update the app to edit it.") : L("Use a hue from 0 to less than 360 and colors in #RRGGBB format. Check every edited state before saving.")}</div></Show>
      <Show when={saveError()}>{(error) => <div class="look-save-error" role="alert">{error()}</div>}</Show>
    </Sheet>
  );
}
