// A bot's look slides in from Details: the generated appearance (a base and one override per
// activity state) and an optional photo. Everything is a draft until Save, which sends one
// `bots.update`; Cancel leaves the account untouched. The photo travels as an encrypted `file`
// blob like an attachment.

import { botAppearanceContrast, resolveBotAppearance } from "@beans/blobatar";
import { ImageManipulator, SaveFormat } from "expo-image-manipulator";
import * as ImagePicker from "expo-image-picker";
import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useNavigation, usePreventRemove } from "expo-router/react-navigation";
import { useEffect, useState } from "react";
import { Alert, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine, type PickedFile } from "../../../src/core/engine";
import { AVATAR_STATES, BACKGROUNDS, EXPRESSIONS, HUE_STEPS, SHAPES, TONES, resolvedContrastWarnings, normalizeHex, setColor, setField, type BotAppearance, type BotLook, type BotPalette } from "../../../src/core/look";
import { useBotMap, useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { AvatarDisc, useBotAvatarUri } from "../../../src/ui/Avatar";
import { FieldRow, Row, Section, type MenuChoice } from "../../../src/ui/forms";
import { backgroundLabels, expressionLabels, shapeLabels, stateLabels, toneLabels } from "../../../src/ui/lookLabels";
import { isDirty, resetScope, restoreDefaults, savePayload, shuffle, startDraft, type LookDraft } from "../../../src/ui/lookDraft";
import { FormToolbar } from "../../../src/ui/navigation";
import { usePalette } from "../../../src/ui/theme";
import { notifyAvatarScroll } from "../../../src/ui/avatarVisibility";
import { useReduceMotion, useScreenFocused } from "../../../src/ui/avatarPrefs";

export default function BotLookScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const bot = useBotMap().get(id);
  const savedUri = useBotAvatarUri(bot);
  const [draft, setDraft] = useState<LookDraft>(() => startDraft(bot?.look));
  const [saving, setSaving] = useState(false);
  // Bumped when the draft changes under the hex fields (scope, Shuffle, Reset), so they re-read it.
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [invalidColors, setInvalidColors] = useState<Partial<Record<keyof BotPalette, boolean>>>({});
  const [cycle, setCycle] = useState(false);
  const [cycleState, setCycleState] = useState(0);
  const [previewVisible, setPreviewVisible] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const active = useStore((s) => s.appActive);
  const focused = useScreenFocused();
  const reduceMotion = useReduceMotion();
  useEffect(() => {
    if (!cycle || !previewVisible || !active || !focused || reduceMotion) return;
    const timer = setInterval(() => setCycleState((current) => (current + 1) % AVATAR_STATES.length), 1500);
    return () => clearInterval(timer);
  }, [cycle, previewVisible, active, focused, reduceMotion]);
  const navigation = useNavigation();
  const replace = (change: (current: LookDraft) => LookDraft) => {
    setDraft(change);
    setRevision((n) => n + 1);
    setInvalidColors({});
  };
  // Saving keeps the draft on-screen until the single update finishes. Cancel and system back
  // share one discard confirmation; confirmed completion clears the guard before navigating.
  const guarded = !!bot && !leaving && (saving || isDirty(draft, bot.look));
  usePreventRemove(guarded, ({ data }) => {
    if (saving) return;
    Alert.alert(t("Discard changes?"), t("Your changes to this look will be lost."), [
      { text: t("Keep Editing"), style: "cancel" },
      { text: t("Discard"), style: "destructive", onPress: () => navigation.dispatch(data.action) },
    ]);
  });
  useEffect(() => { if (leaving) router.back(); }, [leaving, router]);

  if (!bot) return null;

  const dirty = isDirty(draft, bot.look);
  const { scope, look, photo } = draft;
  const inBase = scope === "base";
  // The base previews on its own; a state previews with its overrides over the base.
  const previewState = cycle ? AVATAR_STATES[cycleState] : inBase ? "idle" : scope;
  const previewLook = cycle ? look : inBase ? { ...look, states: undefined } : look;
  const preview = resolveBotAppearance(bot.id, previewLook, previewState);
  const here: BotAppearance = (inBase ? look.base : look.states?.[scope]) ?? {};
  const photoUri = photo.kind === "replace" ? photo.file.uri : photo.kind === "remove" ? undefined : savedUri;
  const hasPhoto = photo.kind === "replace" || (photo.kind === "keep" && !!bot.avatar);
  const warnings = resolvedContrastWarnings(preview, botAppearanceContrast(preview));
  const states = stateLabels();

  const edit = (next: BotLook) => setDraft((current) => ({ ...current, look: next }));
  const field = <K extends Exclude<keyof BotAppearance, "palette">>(key: K, value: BotAppearance[K] | undefined) => edit(setField(look, scope, key, value));

  /// A menu row for one field. The first choice clears it: the seeded default in the base, the
  /// base's value in a state.
  function pick<K extends Exclude<keyof BotAppearance, "palette">>(title: string, key: K, options: readonly NonNullable<BotAppearance[K]>[], label: (value: NonNullable<BotAppearance[K]>) => string) {
    const current = here[key];
    const choices: MenuChoice[] = [
      { title: inBase ? t("Default") : t("Same as Base"), selected: current === undefined, onPress: () => field(key, undefined), dividerAfter: true },
      ...options.map((value) => ({ title: label(value), selected: current === value, onPress: () => field(key, value) })),
    ];
    const value = current === undefined ? (inBase ? t("Default") : t("Same as Base")) : label(current as NonNullable<BotAppearance[K]>);
    return <Row title={title} menu={{ value, title, choices }} />;
  }

  async function choosePhoto() {
    const result = await ImagePicker.launchImageLibraryAsync({ mediaTypes: ["images"], quality: 1 });
    if (result.canceled) return;
    try {
      const file = await squareAvatar(result.assets[0]);
      setDraft((current) => ({ ...current, photo: { kind: "replace", file } }));
    } catch (failure) {
      Alert.alert(t("Could not use that photo"), failure instanceof Error ? failure.message : String(failure));
    }
  }

  async function save() {
    if (!bot || saving || Object.values(invalidColors).some(Boolean)) return;
    const change = savePayload(draft, bot.look);
    if (Object.keys(change).length === 0) {
      setLeaving(true);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await engine.saveBotAppearance(bot.id, change);
      setLeaving(true);
    } catch (failure) {
      // The draft stays as it was so the user can try again or cancel.
      setSaving(false);
      setError(failure instanceof Error ? failure.message : String(failure));
    }
  }

  function cancel() {
    router.back();
  }

  const scopeChoices: MenuChoice[] = [
    { title: t("Base"), selected: inBase, onPress: () => replace((current) => ({ ...current, scope: "base" })), dividerAfter: true },
    ...AVATAR_STATES.map((state) => ({ title: states[state], selected: scope === state, onPress: () => replace((current) => ({ ...current, scope: state })) })),
  ];
  const scopeName = inBase ? t("Base") : states[scope];

  return (
    <>
      <Stack.Screen options={{ title: t("Look"), headerBackVisible: false, gestureEnabled: !dirty && !saving }} />
      <FormToolbar cancelLabel={t("Cancel")} saveLabel={t("Save")} saveDisabled={saving || !dirty || Object.values(invalidColors).some(Boolean)} onCancel={cancel} onSave={() => void save()} />
      <ScrollView pointerEvents={saving ? "none" : "auto"} onScroll={notifyAvatarScroll} scrollEventThrottle={32} contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        <View style={styles.hero} accessible accessibilityLabel={t("Preview of {name}'s look, {state}", { name: bot.name, state: cycle ? states[previewState] : scopeName })}>
          <AvatarDisc name={bot.id} size={96} look={previewLook} state={previewState} onVisibilityChange={setPreviewVisible} />
          {hasPhoto ? <><AvatarDisc name={bot.id} uri={photoUri} size={40} look={look} /><Text style={[styles.note, { color: p.secondaryLabel }]}>{t("The photo shows until you remove it. Your look changes apply once it is gone.")}</Text></> : null}
        </View>
        {error ? <Text style={[styles.error, { color: p.red }]} accessibilityRole="alert">{t("Could not save the look: {error}", { error })}</Text> : null}

        <Section title={t("Editing")} footer={inBase ? t("The base look applies in every state unless a state changes it.") : t("Only what you set here differs from the base.")}>
          <Row title={t("Look for")} menu={{ value: scopeName, title: t("Look for"), choices: scopeChoices }} />
          <Row title={t("Preview Activity Cycle")} menu={{ title: t("Preview Activity Cycle"), value: cycle ? t("On") : t("Off"), choices: [true, false].map((value) => ({ title: value ? t("On") : t("Off"), selected: cycle === value, onPress: () => { setCycle(value); setCycleState(0); } })) }} />
        </Section>

        <Section title={t("Shape and Face")}>
          {pick(t("Shape"), "shape", SHAPES, (value) => shapeLabels()[value])}
          {pick(t("Expression"), "expression", EXPRESSIONS, (value) => expressionLabels()[value])}
          {pick(t("Background"), "background", BACKGROUNDS, (value) => backgroundLabels()[value])}
          {pick(t("Motion"), "motion", [true, false], (value) => (value ? t("On") : t("Off")))}
        </Section>

        <Section title={t("Color")} footer={t("Your own colors are used as chosen, even when the contrast is low.")}>
          {pick(t("Tone"), "tone", TONES, (value) => toneLabels()[value])}
          {pick(t("Hue"), "hue", HUE_STEPS, (value) => `${value}°`)}
          <HexRow key={`head-${revision}`} label={t("Body")} channel="head" palette={here.palette} onValidityChange={(invalid) => setInvalidColors((current) => ({ ...current, head: invalid }))} onChange={(hex) => edit(setColor(look, scope, "head", hex))} />
          <HexRow key={`eye-${revision}`} label={t("Eyes")} channel="eye" palette={here.palette} onValidityChange={(invalid) => setInvalidColors((current) => ({ ...current, eye: invalid }))} onChange={(hex) => edit(setColor(look, scope, "eye", hex))} />
          <HexRow key={`bg-${revision}`} label={t("Background")} channel="bg" palette={here.palette} onValidityChange={(invalid) => setInvalidColors((current) => ({ ...current, bg: invalid }))} onChange={(hex) => edit(setColor(look, scope, "bg", hex))} />
        </Section>
        {warnings.length ? (
          <Text style={[styles.warning, { color: p.red }]} accessibilityRole="alert">
            {warnings.includes("eye") ? t("The eye color is hard to see against the body color.") : ""}
            {warnings.length === 2 ? " " : ""}
            {warnings.includes("bg") ? t("The body color is hard to see against the background.") : ""}
          </Text>
        ) : null}

        <Section>
          <Row title={t("Shuffle")} icon="dice" onPress={() => replace((current) => shuffle(current))} />
          {!inBase ? <Row title={t("Same as Base")} icon="arrow.uturn.backward" onPress={() => replace((current) => resetScope(current))} /> : null}
          <Row title={t("Restore Default Look")} icon="arrow.counterclockwise" destructive onPress={() => replace((current) => restoreDefaults(current))} />
        </Section>

        <Section title={t("Photo")} footer={hasPhoto ? t("The photo shows in place of the generated look, on every paired Device.") : t("A photo shows in place of the generated look. It is shared encrypted, like an attachment.")}>
          <Row title={hasPhoto ? t("Change Photo") : t("Choose Photo")} icon="photo.on.rectangle" onPress={() => void choosePhoto()} />
          {hasPhoto ? <Row title={t("Remove Photo")} icon="xmark.circle.fill" destructive onPress={() => setDraft((current) => ({ ...current, photo: { kind: "remove" } }))} /> : null}
        </Section>
      </ScrollView>
    </>
  );
}

/// One explicit color as hex text: a valid entry sets it, an empty one clears it back to the
/// generated color, anything else waits and is flagged.
function HexRow({ label, channel, palette, onChange, onValidityChange }: { label: string; channel: keyof BotPalette; palette?: BotPalette; onChange: (hex: string | undefined) => void; onValidityChange: (invalid: boolean) => void }) {
  const p = usePalette();
  const [text, setText] = useState(palette?.[channel] ?? "");
  const invalid = text.trim() !== "" && normalizeHex(text) === undefined;
  return (
    <FieldRow
      label={label}
      value={text}
      placeholder={t("Automatic")}
      autoCapitalize="characters"
      autoCorrect={false}
      maxLength={7}
      accessibilityLabel={t("{name} color, hex", { name: label })}
      onChangeText={(next) => {
        setText(next);
        onValidityChange(next.trim() !== "" && normalizeHex(next) === undefined);
        if (next.trim() === "") onChange(undefined);
        else {
          const hex = normalizeHex(next);
          if (hex) onChange(hex);
        }
      }}
      style={invalid ? { color: p.red } : undefined}
    />
  );
}

/// Longest side of a stored profile image, the Mac app's figure too.
const AVATAR_SIDE = 512;

/// The system picker (instant, needs no permission), then a square center crop at `AVATAR_SIDE`
/// px made here: the picker's own crop step is the legacy controller, which takes seconds to
/// appear, and a full photo is far more than an avatar needs to sync.
async function squareAvatar(asset: ImagePicker.ImagePickerAsset): Promise<PickedFile> {
  const side = Math.min(asset.width, asset.height);
  const context = ImageManipulator.manipulate(asset.uri);
  context.crop({ originX: Math.floor((asset.width - side) / 2), originY: Math.floor((asset.height - side) / 2), width: side, height: side });
  if (side > AVATAR_SIDE) context.resize({ width: AVATAR_SIDE, height: AVATAR_SIDE });
  const rendered = await context.renderAsync();
  const saved = await rendered.saveAsync({ format: SaveFormat.JPEG, compress: 0.85 });
  rendered.release();
  context.release();
  const base = (asset.fileName ?? "Photo").replace(/\.[^.]+$/, "");
  return { uri: saved.uri, name: `${base}.jpg`, mime: "image/jpeg", width: saved.width, height: saved.height };
}

const styles = StyleSheet.create({
  hero: { alignItems: "center", paddingTop: 16, paddingBottom: 8, paddingHorizontal: 32, gap: 8 },
  note: { fontSize: 13, textAlign: "center" },
  error: { fontSize: 14, paddingHorizontal: 32, paddingVertical: 8, textAlign: "center" },
  warning: { fontSize: 13, paddingHorizontal: 32, paddingTop: 8 },
});
