// Settings › Search: what a Runner finds messages and pictures with by meaning. Text search runs a
// local model downloaded to the Runner, or an embedding provider the user configured; pictures
// are found by a local image model, and by what a vision model says they show. Anything that
// sends messages or pictures off the Runner asks first.
//
// No model ships with the app and none downloads on its own: the Runner suggests the catalog
// model that suits its computer and chats, lists it first, and downloads a model only after the
// user approves its preview.

import { createEffect, createSignal, For, onSettled, Show } from "solid-js";
import { L } from "../../l10n";
import { canDownload, downloadOrder, modelSummary, modelTooltip, previewSummary, progressText } from "../../model/localModels";
import { providerName, withCustomModels, type Device, type ProviderKind } from "../../model/models";
import { errorText, store } from "../../model/store";
import type { WireInstallProgress, WireLocalModel, WireLocalModels, WireSearchIndexStatus, WireSearchRole, WireTextSource } from "../../model/wire";
import { alert } from "../overlay";
import { ActionRow, KeyValueRow, NoteRow, PopUpRow, Section } from "../sections";
import { Footnote, PaneFrame, pickedDevice, Placeholder } from "./panes";
import { Entries } from "./search";

/** How often the pane asks again while indexing or loading moves. */
const REFRESH_MS = 5000;
/** How often a download under way reports its bytes. */
const PROGRESS_MS = 1000;

export function SearchPane() {
  const device = pickedDevice;
  return (
    <PaneFrame pane="search">
      <Placeholder device={device()}>
        <Show when={device()}>{(runner) => <SearchSettings device={runner()} />}</Show>
      </Placeholder>
      <Footnote
        text={L(
          "Search finds messages by meaning as well as by their words: the Runner indexes their text and the names of their files, never tool output. Local models run on the Runner, and nothing leaves it; a provider or a describer receives what it indexes.",
        )}
      />
    </PaneFrame>
  );
}

/** A role's state in words, with its progress when it is indexing. */
function progress<T>(role: WireSearchRole<T>, noun: "messages" | "pictures"): string {
  switch (role.state) {
    case "off":
      return L("Off");
    case "loading":
    case "not_loaded":
      return L("Loading the model…");
    case "failed":
      return L("Couldn't load the model: %@", role.error ?? "");
    case "ready":
      if (noun === "pictures") return role.pending > 0 ? L("%d pictures indexed, %d waiting", role.indexed, role.pending) : L("%d pictures indexed", role.indexed);
      return role.pending > 0 ? L("%d messages indexed, %d waiting", role.indexed, role.pending) : L("%d messages indexed", role.indexed);
    default:
      return role.state;
  }
}

function textKey(choice: WireTextSource | null): string {
  if (!choice) return "off";
  return choice.kind === "local" ? `local:${choice.model_id}` : `provider:${choice.profile_id}`;
}

export function SearchSettings(props: { device: Device }) {
  const [status, setStatus] = createSignal<WireSearchIndexStatus | null>(null);
  const [catalog, setCatalog] = createSignal<WireLocalModel[]>([]);
  const [recommended, setRecommended] = createSignal<WireLocalModels["recommended"]>(undefined);
  const [failure, setFailure] = createSignal<string | null>(null);
  /** The model being downloaded, whose row shows a spinner. */
  const [installing, setInstalling] = createSignal<string | null>(null);
  /** The Runner's report on the download under way, polled while one is. */
  const [download, setDownload] = createSignal<WireInstallProgress | null>(null);
  let loads = 0;
  const load = async () => {
    const runnerID = props.device.id;
    const load = ++loads;
    try {
      const [next, models] = await Promise.all([store.searchIndexStatus(runnerID), store.localModels(runnerID)]);
      if (load !== loads) return;
      setStatus(next);
      setCatalog(models.models);
      setRecommended(models.recommended);
      setFailure(null);
    } catch (error) {
      if (load === loads) setFailure(errorText(error));
    }
  };
  createEffect(
    () => props.device.id,
    () => {
      setStatus(null);
      void load();
    },
  );
  // Only while something moves: a model loading or a queue with work. A Runner elsewhere is
  // asked through the relay, so an idle pane asks nothing.
  const moving = () => {
    const current = status();
    if (!current) return false;
    return [current.text, current.image].some((role) => role.state === "loading" || role.state === "not_loaded" || (role.state === "ready" && role.pending > 0))
      || (current.describe.choice !== null && current.describe.pending > 0);
  };
  onSettled(() => {
    const timer = setInterval(() => {
      if (moving() && installing() === null) void load();
    }, REFRESH_MS);
    return () => clearInterval(timer);
  });

  const choose = async (choice: Record<string, unknown>, confirmExternal = false) => {
    try {
      setStatus(await store.chooseSearch(props.device.id, choice, confirmExternal));
    } catch (error) {
      void alert({ message: L("Couldn't change search"), informative: errorText(error) });
    }
  };

  const installed = (kind: "text" | "image") => catalog().filter((model) => model.kind === kind && model.installed);
  const downloadable = (kind: "text" | "image") => downloadOrder(catalog().filter((model) => model.kind === kind), recommended()?.[kind]);
  /** The suggested model for `kind` when it still has to be downloaded. */
  const suggestion = (kind: "text" | "image") => catalog().find((model) => model.kind === kind && !model.installed && model.id === recommended()?.[kind]);
  const install = async (model: WireLocalModel) => {
    const runnerID = props.device.id;
    setInstalling(model.id);
    try {
      const preview = await store.previewLocalModel(runnerID, model.id);
      const answer = await alert({
        message: L("Download “%@”?", preview.name),
        informative: previewSummary(preview, props.device.name),
        buttons: [{ title: L("Download") }, { title: L("Cancel") }],
      });
      if (answer !== 0) return;
      // The download answers when it ends; meanwhile its row shows how far it has come.
      const poll = setInterval(() => {
        store.localModelProgress(runnerID).then(setDownload, () => {});
      }, PROGRESS_MS);
      try {
        await store.installLocalModel(runnerID, preview);
      } finally {
        clearInterval(poll);
        setDownload(null);
      }
      await load();
    } catch (error) {
      // A cancel the user asked for is not a failure to report.
      if (!(error instanceof Error && error.message === "install_cancelled")) {
        void alert({ message: L("Couldn't download “%@”", model.name), informative: errorText(error) });
      }
    } finally {
      setInstalling(null);
    }
  };
  const cancelInstall = (model: WireLocalModel) => {
    void store.cancelLocalModelInstall(props.device.id, model.id).catch((error) => {
      void alert({ message: L("Couldn't cancel the download"), informative: errorText(error) });
    });
  };

  const chooseText = async (value: string) => {
    if (value === "off") return choose({ text: null });
    if (value.startsWith("local:")) return choose({ text: { kind: "local", model_id: value.slice("local:".length) } });
    const answer = await alert({
      message: L("Send your messages to this provider?"),
      informative: L("Every message %@ indexes goes to the embedding provider, now and as new ones arrive. A local model keeps them on the Runner.", props.device.name),
      buttons: [{ title: L("Use Provider") }, { title: L("Cancel") }],
      style: "warning",
    });
    if (answer === 0) await choose({ text: { kind: "provider", profile_id: value.slice("provider:".length) } }, true);
  };

  /** Connected providers' models that take images, for describing pictures. */
  const visionModels = () =>
    withCustomModels(store.models, store.providers).filter((model) => model.images && store.providers.some((provider) => provider.kind === model.provider && provider.isConnected));
  const describeKey = () => {
    const choice = status()?.describe.choice;
    return choice ? `${choice.provider}\u0000${choice.model}` : "off";
  };
  const chooseDescribe = async (value: string) => {
    if (value === "off") return choose({ describe: null });
    const [provider, model] = value.split("\u0000") as [string, string];
    const answer = await alert({
      message: L("Send your pictures to %@?", providerName(provider as ProviderKind, store.providers)),
      informative: L("Each picture in your chats is sent to this model once, to describe what it shows. The tokens count toward the chat's spending."),
      buttons: [{ title: L("Describe Pictures") }, { title: L("Cancel") }],
      style: "warning",
    });
    if (answer === 0) await choose({ describe: { provider, model } }, true);
  };

  const DownloadRows = (rowProps: { kind: "text" | "image" }) => (
    <>
      <Show when={suggestion(rowProps.kind)}>
        {(model) => <NoteRow text={L("Suggested for %@: %@. Nothing downloads until you approve it.", props.device.name, model().name)} tint="var(--label-2)" />}
      </Show>
      <For each={downloadable(rowProps.kind)} keyed={(model) => model.id}>
        {(model) => (
          <ActionRow
            label={model().name}
            value={progressText(download(), model().id) ?? modelSummary(model(), model().id === recommended()?.[rowProps.kind])}
            tint={model().id === recommended()?.[rowProps.kind] ? undefined : "var(--label-2)"}
            tooltip={modelTooltip(model())}
            actionTitle={canDownload(model()) ? L("Download…") : undefined}
            busy={installing() === model().id}
            onAction={() => void install(model())}
            secondActionTitle={installing() === model().id && download()?.installing?.model_id === model().id ? L("Cancel") : undefined}
            onSecondAction={() => cancelInstall(model())}
          />
        )}
      </For>
    </>
  );

  return (
    <Show when={status()} fallback={<KeyValueRow label={failure() ?? L("Loading…")} value="" tint={failure() ? "var(--red)" : "var(--label-2)"} />}>
      {(current) => (
        <>
          <Section title={L("Messages")} style="heading">
            <PopUpRow
              label={Entries.searchText().row}
              value={textKey(current().text.choice)}
              onChange={(value) => void chooseText(value)}
              options={[
                { value: "off", label: L("Off") },
                ...installed("text").map((model, index) => ({ value: `local:${model.id}`, label: model.name, separated: index === 0 })),
                ...current().profiles.map((profile, index) => ({ value: `provider:${profile.id}`, label: L("%@ (provider)", profile.model), separated: index === 0 })),
              ]}
            />
            <KeyValueRow label={L("Index")} value={progress(current().text, "messages")} tint={current().text.state === "failed" ? "var(--red)" : undefined} />
            <DownloadRows kind="text" />
          </Section>
          <Section title={L("Pictures")} style="heading">
            <PopUpRow
              label={Entries.searchImages().row}
              value={current().image.choice ?? "off"}
              onChange={(value) => void choose({ image_model_id: value === "off" ? null : value })}
              options={[{ value: "off", label: L("Off") }, ...installed("image").map((model, index) => ({ value: model.id, label: model.name, separated: index === 0 }))]}
            />
            <KeyValueRow label={L("Index")} value={progress(current().image, "pictures")} tint={current().image.state === "failed" ? "var(--red)" : undefined} />
            <DownloadRows kind="image" />
            <PopUpRow
              label={Entries.describeImages().row}
              value={describeKey()}
              onChange={(value) => void chooseDescribe(value)}
              options={[
                { value: "off", label: L("Off") },
                ...visionModels().map((model, index) => ({
                  value: `${model.provider}\u0000${model.id}`,
                  label: `${providerName(model.provider as ProviderKind, store.providers)} · ${model.label}`,
                  separated: index === 0,
                })),
              ]}
            />
            <Show when={current().describe.choice && current().describe.pending > 0}>
              <KeyValueRow label={L("Waiting to be described")} value={L("%d messages", current().describe.pending)} tint="var(--label-2)" />
            </Show>
            <Show when={!current().runs_models}>
              <NoteRow text={L("This Runner's build cannot run local models.")} tint="var(--red)" />
            </Show>
          </Section>
        </>
      )}
    </Show>
  );
}
