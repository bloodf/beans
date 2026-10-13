// A custom provider, slid in from Settings inside the same sheet: any server that speaks
// OpenAI's or Anthropic's API, such as a gateway or a model server on the user's network. With a
// `preset` the form starts from a server people often add; with a provider's `kind` it edits that
// one; with neither it starts empty. The core lists the server's models as the URL and the key
// change, and reaches the server again before it saves; the provider joins the account's
// encrypted credentials, which every paired Device shares.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ActivityIndicator, Alert, ScrollView, StyleSheet, Text, View } from "react-native";
import { randomUUID } from "expo-crypto";
import { engine } from "../../src/core/engine";
import {
  CUSTOM_APIS,
  customAPI,
  customPreset,
  customRequestURL,
  defaultModelId,
  defaultProviderName,
  isCustomProvider,
  isHTTPURL,
  isLoopbackHost,
  modelLabel,
  isRunner,
  modelListingNote,
  presetForURL,
  savedModelRows,
  selectedModelIds,
  urlHost,
  type CustomAPI,
} from "../../src/core/model";
import { useStore } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { FieldRow, Row, Section } from "../../src/ui/forms";
import { chooseDefaultModel, setModelListing, startModelDraft, takeModelListing, useModelDraft } from "../../src/ui/modelDraft";
import { usePalette } from "../../src/ui/theme";

// Model checks run only after an explicit user action.

function param(value: string | string[] | undefined): string | undefined {
  return (Array.isArray(value) ? value[0] : value) || undefined;
}

function messageOf(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

export default function CustomProviderScreen() {
  useLanguage();
  const params = useLocalSearchParams<{ kind?: string; preset?: string }>();
  const kind = param(params.kind);
  const router = useRouter();
  const p = usePalette();
  const status = useStore((s) => (kind ? s.providers.find((provider) => provider.kind === kind) : undefined));
  // The provider as it stood when the form opened. The fields start from it, and the form stays
  // as it is while a delete started here takes the provider out of the store.
  const [saved] = useState(status);
  const [saveKind] = useState(() => kind ?? `custom:setup-${randomUUID().toLowerCase()}`);
  const adding = kind ? undefined : customPreset(param(params.preset));
  const integration = saved?.integration;
  const simpleSetup = !saved && adding?.compatible === true;
  const [customize, setCustomize] = useState(!simpleSetup);
  const autoSelect = useRef(!saved);
  const [name, setName] = useState(saved?.name ?? adding?.name ?? "");
  const [api, setAPI] = useState<CustomAPI>(customAPI(saved?.api ?? adding?.api).id);
  const [baseURL, setBaseURL] = useState(saved?.base_url ?? adding?.baseURL ?? "");
  const [apiKey, setAPIKey] = useState("");
  const devices = useStore((s) => s.devices);
  const runners = devices.filter(isRunner);
  const [runnerID, setRunnerID] = useState(() => runners.find((device) => device.status === "online")?.id ?? "");
  const runner = runners.find((device) => device.id === runnerID);
  const runnerReady = !!runner && runner.status === "online";
  const [contextWindow, setContextWindow] = useState(saved?.capabilities?.context_window?.toString() ?? "");
  const [images, setImages] = useState<boolean | null>(saved?.capabilities?.images ?? null);
  const [tools, setTools] = useState<boolean | null>(saved?.capabilities?.tools ?? null);
  const capabilities = { context_window: contextWindow.trim() ? Number(contextWindow) : null, images, tools };
  const validWindow = !contextWindow.trim() || (/^[0-9]+$/.test(contextWindow) && Number.isSafeInteger(Number(contextWindow)) && Number(contextWindow) > 0);
  const [checkRevision, setCheckRevision] = useState(0);
  const [checked, setChecked] = useState(false);
  // An edited provider's key comes from the core before the server is first asked for models.
  const [keyLoaded, setKeyLoaded] = useState(!saved);
  const [working, setWorking] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshNote, setRefreshNote] = useState<string | null>(null);
  const savedKey = useRef<string | null>(null);
  const [initialRows] = useState(() => savedModelRows(saved?.models ?? []));
  const rows = useModelDraft((s) => s.rows);
  const chosenDefault = useModelDraft((s) => s.chosenDefault);
  const listing = useModelDraft((s) => s.listing);
  /// The latest ask for the server's models; an answer to an older one is dropped.
  const asked = useRef(0);
  /// Past the first ask, which goes at once: the later ones wait for the typing to pause.
  const opened = useRef(false);

  // The picker this form pushes works on the same rows.
  const [startsWithURL] = useState(() => isHTTPURL(baseURL));
  useLayoutEffect(() => startModelDraft(initialRows, startsWithURL), [initialRows, startsWithURL]);

  // A status carries only a masked key: the form asks for the saved one, and keeps what the user
  // typed meanwhile.
  useEffect(() => {
    if (!saved) return;
    let current = true;
    engine.providerAPIKey(saved.kind).then(({ api_key }) => {
      if (!current) return;
      savedKey.current = api_key ?? "";
      setAPIKey((typed) => typed || api_key || "");
      setKeyLoaded(true);
    }).catch((cause) => {
      if (current) setError(messageOf(cause));
    });
    return () => {
      current = false;
    };
  }, [saved]);

  // Field changes revoke the previous check and invalidate late replies.
  useEffect(() => () => void (asked.current += 1), []);
  useEffect(() => {
    ++asked.current;
    setChecked(false);
    setModelListing({ state: "none" });
  }, [api, baseURL, apiKey, runnerID, contextWindow, images, tools]);
  useEffect(() => {
    if (!checkRevision || !keyLoaded || !runnerReady || !validWindow || !isHTTPURL(baseURL)) return;
    const ask = ++asked.current;
    setModelListing({ state: "loading" });
    engine.listCustomModels({ name: name.trim() || defaultProviderName(baseURL), api, baseURL, apiKey, integration, runnerID, capabilities }).then(({ listed, models }) => {
      if (ask !== asked.current) return;
      takeModelListing(listed, models, autoSelect.current);
      if (listed && models.length) autoSelect.current = false;
      setChecked(true);
    }).catch((cause) => {
      if (ask === asked.current) setModelListing({ state: "error", message: messageOf(cause) });
    });
  }, [checkRevision]);

  if (kind && (!saved || !isCustomProvider(kind))) {
    return (
      <>
        <Stack.Screen options={{ title: t("Provider") }} />
        <View style={styles.center}>
          <Text style={{ color: p.secondaryLabel }}>{t("Unknown provider")}</Text>
        </View>
      </>
    );
  }

  const protocol = customAPI(api);
  const host = urlHost(baseURL);
  // The preset being added, else the one whose server the URL names or whose name the provider
  // has: for the key's hint.
  const preset = adding ?? presetForURL(baseURL) ?? customPreset(saved?.name);
  const requestURL = customRequestURL(api, baseURL);
  // Loopback resolves on the selected Runner, never the phone.
  const local = !!adding?.local || isLoopbackHost(host);
  const urlNote = [
    requestURL ? t("Requests go to {url}.", { url: requestURL }) : t("Beans adds {path} to it.", { path: protocol.path }),
    local ? t("Use the address of the computer running it, as your Runners reach it.") : undefined,
    t("Model listing checks reachability, not inference access. Each Runner must reach this URL."),
  ]
    .filter(Boolean)
    .join("\n");
  const picked = rows.filter((row) => row.selected);
  const defaultId = defaultModelId(rows, chosenDefault);
  const defaultRow = picked.find((row) => row.id === defaultId);
  // Left empty, the name is the preset's whose server the URL names, else the host.
  const fallbackName = defaultProviderName(baseURL);
  const providerName = name.trim() || fallbackName;
  const canSave = !working && keyLoaded && runnerReady && validWindow && checked && !!providerName && isHTTPURL(baseURL) && picked.length > 0;

  async function save() {
    if (!canSave) return;
    setWorking(true);
    setError(null);
    try {
      await engine.saveCustomProvider({ kind: saveKind, integration, name: providerName, api, baseURL, apiKey, models: selectedModelIds(rows, chosenDefault), runnerID, capabilities });
      router.back();
    } catch (cause) {
      setError(messageOf(cause));
    } finally {
      setWorking(false);
    }
  }

  async function refreshModels() {
    if (!kind || working || !keyLoaded || !runnerReady) return;
    ++asked.current;
    setWorking(true);
    setRefreshing(true);
    setError(null);
    setRefreshNote(null);
    const before = useStore.getState().providers.find((provider) => provider.kind === kind)?.models;
    try {
      await engine.refreshCustomModels(runnerID, kind);
      const after = useStore.getState().providers.find((provider) => provider.kind === kind)?.models;
      setRefreshNote(JSON.stringify(before) === JSON.stringify(after) ? t("Models unchanged") : t("Models updated"));
    } catch (cause) {
      setError(messageOf(cause));
    } finally {
      const after = useStore.getState().providers.find((provider) => provider.kind === kind)?.models;
      if (after && api === saved?.api && baseURL.trim() === saved?.base_url && apiKey === savedKey.current) takeModelListing(true, after);
      setRefreshing(false);
      setWorking(false);
    }
  }

  function confirmDelete() {
    if (!kind) return;
    Alert.alert(t("Delete {name}?", { name: saved?.name || providerName }), t("This removes the provider from every paired Device."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Delete"),
        style: "destructive",
        onPress: () => {
          setWorking(true);
          setError(null);
          void engine
            .disconnectProvider(kind)
            .then(() => router.back())
            .catch((cause) => setError(messageOf(cause)))
            .finally(() => setWorking(false));
        },
      },
    ]);
  }

  return (
    <>
      <Stack.Screen options={{ title: saved?.name || (adding ? t("Add {name}", { name: adding.name }) : t("Add Custom Provider")) }} />
      <ScrollView
        contentInsetAdjustmentBehavior="automatic"
        automaticallyAdjustKeyboardInsets
        contentContainerStyle={styles.content}
        keyboardDismissMode="on-drag"
        keyboardShouldPersistTaps="handled"
      >
        <Section footer={t("Loopback belongs to the selected Runner. For another host, use its reachable address and check bind address, firewall and proxy bypass. Do not expose an unauthenticated server publicly.")}>
          <Row title={t("Check from Runner")} menu={{ title: t("Check from Runner"), value: runner?.name ?? t("None"), choices: working ? [] : runners.map((device) => ({ title: device.name, selected: device.id === runnerID, onPress: () => setRunnerID(device.id) })) }} />
          <Row title={t("Check Connection")} onPress={!working && keyLoaded && runnerReady && validWindow && isHTTPURL(baseURL) ? () => setCheckRevision((value) => value + 1) : undefined} />
          <Text style={{ color: p.secondaryLabel }}>{runner ? t("Requests run on {name}. Listing verifies connectivity/catalog only, not inference.", { name: runner.name }) : t("Pair an online desktop Runner before checking.")}</Text>
        </Section>
        <Section footer={t("Unknown images stay text-only. Unknown tools are unverified. Tools No cannot run Beans tool-bearing bot turns; tool-free inference remains available.")}>
          <FieldRow label={t("Context window (tokens)")} value={contextWindow} onChangeText={setContextWindow} keyboardType="number-pad" editable={!working} />
          {[{ title: t("Image support"), value: images, set: setImages }, { title: t("Tool support"), value: tools, set: setTools }].map((choice) => <Row key={choice.title} title={choice.title} menu={{ title: choice.title, value: choice.value === null ? t("Unknown — use discovery") : choice.value ? t("Yes") : t("No"), choices: [null, true, false].map((value) => ({ title: value === null ? t("Unknown — use discovery") : value ? t("Yes") : t("No"), selected: value === choice.value, onPress: () => { if (!working) choice.set(value); } })) }} />)}
        </Section>
        {customize && <Section footer={t("Any server that speaks OpenAI’s or Anthropic’s API, such as a gateway or a model server on your network. Encrypted and shared with your paired Devices.")}>
          <FieldRow
            label={t("Name")}
            value={name}
            onChangeText={setName}
            placeholder={fallbackName || "OpenRouter"}
            autoCapitalize="words"
            autoCorrect={false}
            editable={!working}
            returnKeyType="next"
          />
          <Row
            title={t("API")}
            menu={{
              title: t("API"),
              value: protocol.title,
              choices: integration ? [] : CUSTOM_APIS.map((choice) => ({ title: choice.title, selected: choice.id === api, onPress: () => setAPI(choice.id) })),
            }}
          />
        </Section>}

        <Section footer={`${urlNote}\n${t("Port suggestions only: 11434 or 1234. Enter your server’s exact URL; Beans never scans ports.")}`}>
          <FieldRow
            label={t("Base URL")}
            value={baseURL}
            onChangeText={setBaseURL}
            placeholder={protocol.placeholder}
            autoCapitalize="none"
            autoCorrect={false}
            keyboardType="url"
            editable={!working}
            returnKeyType="next"
          />
        </Section>

        <Section>
          <FieldRow
            label={t("API Key")}
            value={apiKey}
            onChangeText={setAPIKey}
            placeholder={preset ? preset.keyPlaceholder() : t("Optional for a server on your network")}
            secureTextEntry
            autoCapitalize="none"
            autoCorrect={false}
            editable={!working}
            returnKeyType="done"
          />
        </Section>

        {simpleSetup ? <Section><Row title={t("Advanced")} onPress={() => setCustomize(!customize)} /></Section> : null}
        {customize && <Section footer={modelListingNote(listing)}>
          <Row
            title={t("Models")}
            detail={picked.length ? t("{count} selected", { count: picked.length }) : t("None")}
            chevron
            onPress={() => router.push("/settings/custom-models")}
          />
          {saved?.is_connected ? <Row title={t("Refresh Models")} onPress={!working && keyLoaded ? () => void refreshModels() : undefined} accessory={refreshing ? <ActivityIndicator /> : undefined} /> : null}
          {defaultRow ? (
            <Row
              title={t("Default Model")}
              menu={{
                title: t("Default Model"),
                value: modelLabel(defaultRow),
                choices: picked.map((row) => ({ title: modelLabel(row), selected: row.id === defaultId, onPress: () => chooseDefaultModel(row.id) })),
              }}
            />
          ) : (
            <Row title={t("Default Model")} detail={t("None")} />
          )}
        </Section>}

        <Section>
          <Row title={kind ? t("Save") : t("Add")} onPress={canSave ? () => void save() : undefined} accessory={working && !refreshing ? <ActivityIndicator /> : undefined} />
        </Section>

        {error ? <Text style={[styles.error, { color: p.red }]}>{error}</Text> : null}
        {refreshNote ? <Text style={[styles.error, { color: p.secondaryLabel }]}>{refreshNote}</Text> : null}

        {kind ? (
          <Section>
            <Row title={t("Delete")} destructive onPress={!working ? confirmDelete : undefined} />
          </Section>
        ) : null}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  center: { flex: 1, alignItems: "center", justifyContent: "center" },
  error: { marginHorizontal: 32, marginTop: 10, fontSize: 13, lineHeight: 18 },
});
