import * as Application from "expo-application";
import { AlertDialog, Column, Host, OutlinedTextField, RadioButton, Row as ComposeRow, Text as ComposeText, TextButton } from "@expo/ui/jetpack-compose";
import { clickable, fillMaxWidth, padding } from "@expo/ui/jetpack-compose/modifiers";
import { Stack, useRouter } from "expo-router";
import { useState } from "react";
import { ActivityIndicator, Alert, Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../src/core/engine";
import { CUSTOM_PRESETS, customProviderNamed, deviceName, isCustomProvider, isRunner, presetGroup, reviewModelRows, reviewPicker, withCustomModels, type ReviewModelRow, providerLabel } from "../../src/core/model";
import { deviceIsOnline, useStore } from "../../src/core/store";
import { checkUpdates, dismissUpdate, installUpdate, setUpdateChecks, useUpdates } from "../../src/core/updates";
import { deviceLanguage, languageNames, languages, setAppLanguage, t, useLanguage } from "../../src/i18n";
import { FieldRow, MenuRow, Row, Section, ToggleRow, type MenuChoice } from "../../src/ui/forms";
import { lastSeen } from "../../src/ui/format";
import { Symbol } from "../../src/ui/Symbol";
import { usePalette } from "../../src/ui/theme";
import { useDeviceRows } from "../../src/ui/devices";
import { useWide } from "../../src/ui/layout";
import { CloseToolbar } from "../../src/ui/navigation";
import { DeviceRename } from "../../src/ui/DeviceRename";
import { alert } from "../../src/ui/alert";
import {
  automaticLanguage,
  languageName,
  setDictationLanguage,
  useDictationLanguage,
  useSupportedLanguages,
} from "../../src/ui/dictation";

export default function SettingsScreen() {
  const router = useRouter();
  const p = usePalette();
  const wide = useWide();
  const deviceRows = useDeviceRows();
  const devices = useStore((s) => s.devices);
  const relayConnected = useStore((s) => s.relayConnected);
  const relayUpdateRequired = useStore((s) => s.relayUpdateRequired);
  const relayError = useStore((s) => s.relayError);
  const relayUrl = useStore((s) => s.relayUrl);
  const identity = useStore((s) => s.identityId);
  const autoReview = useStore((s) => s.auto_review);
  const paused = useStore((s) => s.paused);
  const providers = useStore((s) => s.providers);
  const sharedLinks = useStore((s) => s.shared_links);
  const catalog = withCustomModels(useStore((s) => s.models), providers);
  const thisDevice = devices.find((d) => d.is_this_device);
  const [addingRule, setAddingRule] = useState(false);
  const [ruleText, setRuleText] = useState("");
  const [ruleBehavior, setRuleBehavior] = useState<"allow" | "ask">("allow");
  const [savingPause, setSavingPause] = useState(false);
  const dictation = useDictationLanguage();
  const appLanguage = useLanguage();
  const updates = useUpdates();
  const updateReason = updates.reason;

  // A pop-up of Automatic with the language it resolves to, a separator, then every language the
  // recognizer knows, by name.
  const dictationLanguages = useSupportedLanguages();
  const automaticDictation = t("Automatic ({language})", { language: languageName(automaticLanguage()) });
  const dictationChoices = [
    { title: automaticDictation, selected: !dictation.setting, onPress: () => setDictationLanguage(undefined), dividerAfter: true },
    ...dictationLanguages.map((tag) => ({ title: languageName(tag), selected: dictation.setting === tag, onPress: () => setDictationLanguage(tag) })),
  ];
  const systemLanguage = t("System ({language})", { language: languageNames[deviceLanguage()] });
  const appLanguageChoices = [
    { title: systemLanguage, selected: !appLanguage.chosen, onPress: () => setAppLanguage(undefined) },
    ...languages.map((code) => ({ title: languageNames[code], selected: appLanguage.chosen === code, onPress: () => setAppLanguage(code) })),
  ];
  // Reviews with, after Details' Runs with: the bot's own provider, then every connected one.
  const reviewer = reviewPicker(autoReview, providers);
  const botsProvider = t("Bot's provider");
  const reviewProviderChoices: MenuChoice[] = [
    {
      title: botsProvider,
      selected: !reviewer.provider,
      onPress: () => {
        if (autoReview.provider) engine.setReviewProvider(undefined);
      },
      dividerAfter: reviewer.providers.length > 0,
    },
    ...reviewer.providers.map((kind) => ({
      title: providerLabel(kind, providers),
      selected: kind === reviewer.provider,
      onPress: () => {
        if (kind !== autoReview.provider) engine.setReviewProvider(kind);
      },
    })),
  ];
  // Review Models: each connected provider's, Default (the model its status names) first, then
  // all its models, decision models among them.
  const reviewModels = reviewModelRows(autoReview, providers, catalog);
  function reviewModelMenu(row: ReviewModelRow) {
    const fallback = row.defaultName ? t("Default ({model})", { model: row.defaultName }) : t("Default");
    const choices: MenuChoice[] = [
      {
        title: fallback,
        selected: !row.picked,
        onPress: () => {
          if (row.picked) engine.setReviewModel(row.kind, undefined);
        },
        dividerAfter: row.models.length > 0,
      },
      ...row.models.map((model) => ({
        title: model.name,
        selected: model.id === row.picked,
        onPress: () => {
          if (model.id !== row.picked) engine.setReviewModel(row.kind, model.id);
        },
      })),
    ];
    const value = row.picked ? (row.models.find((model) => model.id === row.picked)?.name ?? row.picked) : fallback;
    return { title: providerLabel(row.kind, providers), value, choices };
  }
  // Add Custom Provider's menu: the hosted presets, the ones on the user's own computer, the
  // decision APIs Auto-review can use, then any other server. A preset the account has already
  // opens that provider, checked.
  const customProviderChoices: MenuChoice[] = [
    ...CUSTOM_PRESETS.map((preset, index) => {
      const existing = preset.compatible ? undefined : customProviderNamed(preset.name, providers);
      const next = CUSTOM_PRESETS[index + 1];
      return {
        title: preset.name,
        selected: !!existing,
        dividerAfter: !next || presetGroup(next) !== presetGroup(preset),
        onPress: () =>
          router.push({ pathname: "/settings/custom-provider", params: existing ? { kind: existing.kind } : { preset: preset.name } }),
      };
    }),
    { title: t("Other Server…"), selected: false, onPress: () => router.push("/settings/custom-provider") },
  ];

  function addRule() {
    if (Platform.OS === "android") {
      setRuleText("");
      setRuleBehavior("allow");
      setAddingRule(true);
      return;
    }
    Alert.prompt(t("New rule"), t("When a bot wants to…"), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Allow automatically"),
        onPress: (text?: string) => saveRule(text, "allow"),
      },
      { text: t("Ask first"), onPress: (text?: string) => saveRule(text, "ask") },
    ]);
  }

  function saveRule(text: string | undefined, behavior: "allow" | "ask") {
    const trimmed = (text ?? "").trim();
    if (!trimmed) return;
    engine.setAutoReview({
      ...autoReview,
      rules: [...autoReview.rules, { id: "", text: trimmed, behavior }],
    });
  }

  function editRule(id: string) {
    const rule = autoReview.rules.find((r) => r.id === id);
    if (!rule) return;
    const flipped: "allow" | "ask" =
      rule.behavior === "allow" ? "ask" : "allow";
    alert(
      rule.text,
      rule.behavior === "allow" ? t("Allow automatically") : t("Ask first"),
      [
        { text: t("Cancel"), style: "cancel" },
        {
          text: flipped === "allow" ? t("Allow automatically") : t("Ask first"),
          onPress: () =>
            engine.setAutoReview({
              ...autoReview,
              rules: autoReview.rules.map((r) =>
                r.id === id ? { ...r, behavior: flipped } : r,
              ),
            }),
        },
        {
          text: t("Delete"),
          style: "destructive",
          onPress: () =>
            engine.setAutoReview({
              ...autoReview,
              rules: autoReview.rules.filter((r) => r.id !== id),
            }),
        },
      ],
    );
  }

  function confirmUnpair() {
    alert(
      t("Unpair this phone?"),
      t("Its keys and the synced chats are removed from this phone. Your other paired Devices keep everything, and you can pair again any time."),
      [
        { text: t("Cancel"), style: "cancel" },
        {
          text: t("Unpair"),
          style: "destructive",
          onPress: () => {
            // Forgetting the identity flips `paired`, and the guarded stack swaps to the pair
            // screen on its own; dismissing here would find nothing to pop.
            void engine.unpair();
          },
        },
      ],
    );
  }

  async function changePause(value: boolean) {
    if (savingPause) return;
    setSavingPause(true);
    try {
      await engine.setAccountPaused(value);
    } catch (error) {
      Alert.alert(t("Could not update pause"), error instanceof Error ? error.message : String(error));
    } finally {
      setSavingPause(false);
    }
  }
  return (
    <>
      <Stack.Screen options={{ title: t("Settings") }} />
      <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.dismiss()} />
      <ScrollView
        contentInsetAdjustmentBehavior="automatic"
        contentContainerStyle={{ paddingBottom: 40 }}
        keyboardDismissMode="on-drag"
      >
        <Section
          title={t("This phone")}
          footer={t("How this phone shows up in the Device list on your other Devices.")}
        >
          {thisDevice ? <DeviceRename device={thisDevice} /> : null}
          <Row title={t("Role")} detail={t("Device")} />
          <Row
            title={t("App Language")}
            menu={{ title: t("App Language"), value: appLanguage.chosen ? languageNames[appLanguage.chosen] : systemLanguage, choices: appLanguageChoices }}
          />
          <Row title={t("Diagnostics")} chevron onPress={() => router.push("/settings/diagnostics")} />
        </Section>

        <Section title={t("App updates")} footer={
          updateReason === "development" ? t("Beans Dev does not install release updates.") :
          updateReason === "store_managed" ? t("Use your store or TestFlight to update Beans.") :
          updateReason ? t("Signed Android update verification is unavailable in this build. No APK is downloaded or installed. Use your existing distribution channel.") :
          t("Save or send drafts before installing. Android asks for installation confirmation. Release notes are untrusted text.")
        }>
          <Row title={t("Version")} detail={Application.nativeApplicationVersion ?? "—"} />
          <Row title={t("Build")} detail={Application.nativeBuildVersion ?? "—"} />
          {!updateReason && <>
            <ToggleRow title={t("Check daily")} value={updates.checks} onValueChange={setUpdateChecks} />
            <Row title={t("Check for updates")} detail={updates.status} onPress={() => void checkUpdates()} />
            {updates.status === "downloading" && <Row title={t("Downloading update")} detail={`${Math.round(updates.progress * 100)}%`} />}
            {updates.error && <Row title={t("Update failed")} subtitle={updates.error} subtitleLines={5} />}
            {updates.offer && <>
              <Row title={updates.offer.version} subtitle={updates.offer.notes} subtitleLines={12} />
              <Row title={t("Install Update")} onPress={() => Alert.alert(t("Install Update"), t("Save or send all drafts first. Installing replaces this process. Have you saved your changes?"), [
                { text: t("Later"), style: "cancel", onPress: () => dismissUpdate() },
                { text: t("Install Update"), onPress: () => void installUpdate(updates.offer!.id, true) },
              ])} />
              <Row title={t("Later")} onPress={() => dismissUpdate()} />
              <Row title={t("Skip this version")} onPress={() => dismissUpdate(true)} />
            </>}
          </>}
        </Section>

        <Section title={t("Dictation")}>
          <Row title={t("Language")} menu={{ title: t("Dictation Language"), value: dictation.setting ? languageName(dictation.setting) : automaticDictation, choices: dictationChoices }} />
        </Section>


        <Section>
          <Row
            title={t("Unpair This Phone")}
            icon="xmark"
            destructive
            onPress={confirmUnpair}
          />
        </Section>

        <Text style={[styles.version, { color: p.tertiaryLabel }]}>
          {Application.applicationName ?? "Beans"} {Application.nativeApplicationVersion ?? ""} (
          {Application.nativeBuildVersion ?? ""})
        </Text>
      </ScrollView>
      {Platform.OS === "android" && addingRule ? (
        <Host style={styles.dialogHost} pointerEvents="box-none">
          <AlertDialog onDismissRequest={() => setAddingRule(false)}>
            <AlertDialog.Title>
              <ComposeText style={{ typography: "headlineSmall" }}>{t("New rule")}</ComposeText>
            </AlertDialog.Title>
            <AlertDialog.Text>
              <Column verticalArrangement={{ spacedBy: 12 }} modifiers={[fillMaxWidth()]}>
                <OutlinedTextField autoFocus singleLine onValueChange={setRuleText} modifiers={[fillMaxWidth()]}>
                  <OutlinedTextField.Placeholder>
                    <ComposeText>{t("When a bot wants to…")}</ComposeText>
                  </OutlinedTextField.Placeholder>
                </OutlinedTextField>
                <ComposeRow
                  verticalAlignment="center"
                  modifiers={[fillMaxWidth(), clickable(() => setRuleBehavior("allow")), padding(0, 4, 0, 4)]}
                >
                  <RadioButton selected={ruleBehavior === "allow"} onClick={() => setRuleBehavior("allow")} />
                  <ComposeText>{t("Allow automatically")}</ComposeText>
                </ComposeRow>
                <ComposeRow
                  verticalAlignment="center"
                  modifiers={[fillMaxWidth(), clickable(() => setRuleBehavior("ask")), padding(0, 4, 0, 4)]}
                >
                  <RadioButton selected={ruleBehavior === "ask"} onClick={() => setRuleBehavior("ask")} />
                  <ComposeText>{t("Ask first")}</ComposeText>
                </ComposeRow>
              </Column>
            </AlertDialog.Text>
            <AlertDialog.ConfirmButton>
              <TextButton
                enabled={!!ruleText.trim()}
                onClick={() => {
                  saveRule(ruleText, ruleBehavior);
                  setAddingRule(false);
                }}
              >
                <ComposeText>{t("Create")}</ComposeText>
              </TextButton>
            </AlertDialog.ConfirmButton>
            <AlertDialog.DismissButton>
              <TextButton onClick={() => setAddingRule(false)}>
                <ComposeText>{t("Cancel")}</ComposeText>
              </TextButton>
            </AlertDialog.DismissButton>
          </AlertDialog>
        </Host>
      ) : null}
    </>
  );
}

const styles = StyleSheet.create({
  version: { textAlign: "center", fontSize: 12, marginTop: 28 },
  dialogHost: { position: "absolute", top: 0, right: 0, bottom: 0, left: 0 },
});
