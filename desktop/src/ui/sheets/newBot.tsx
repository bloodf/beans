// New bot form: its ID determines its avatar after creation; the legacy look fields stay on wire.
// It gets one Runner and a direct chat.

import { createMemo, createSignal, Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import { L } from "../../l10n";
import {
  providerKinds,
  providerModels,
  providerName,
  providerSubtitle,
  thinkingLevels,
  withCustomModels,
  type ProviderKind,
} from "../../model/models";
import { track } from "../../model/reactive";
import { store } from "../../model/store";
import { PopUpButton, TextArea, TextField } from "../controls";
import { presentSheet, Sheet } from "../overlay";

/** `onCreate` gets the new bot's id. */
export function presentNewBot(onCreate: (botID: string) => void): void {
  presentSheet((dismiss) => <NewBotSheet onCreate={onCreate} dismiss={dismiss} />);
}

function LabeledRow(props: { label: string; top?: boolean; children: JSX.Element }) {
  return (
    <div class={["labeled-row", { top: !!props.top }]}>
      <span class="labeled-row-label">{props.label}</span>
      <div class="labeled-row-control">{props.children}</div>
    </div>
  );
}

function NewBotSheet(props: { onCreate: (botID: string) => void; dismiss: () => void }) {
  const [name, setName] = createSignal("");
  const [description, setDescription] = createSignal("");
  const [runnerID, setRunnerID] = createSignal(store.runners[0]?.id ?? "");
  const [provider, setProvider] = createSignal<ProviderKind>(providerKinds[0]!);
  const [model, setModel] = createSignal("");
  const [thinking, setThinking] = createSignal("");
  /** The providers as the sheet opened: the built-in ones, then the custom ones. */
  const kinds = store.providerKinds;
  const providers = () => {
    track.roster();
    return store.providers;
  };

  const runners = createMemo(() => {
    track.roster();
    return store.runners;
  });
  const runner = () => runners().find((device) => device.id === runnerID()) ?? runners()[0];
  const note = (): { text: string; warning: boolean } => {
    track.roster();
    const host = runner();
    if (!host) return { text: L("No Runner is paired. Bots run on a Device with macOS, Linux, or Windows."), warning: true };
    const name = providerName(provider(), store.providers);
    if (store.credential(provider())?.isConnected) return { text: L("%@ is connected. Turns run on %@.", name, host.name), warning: false };
    return {
      text: L("%@ is not connected yet. The bot is created now and its first turn waits until you connect it in Settings.", name),
      warning: true,
    };
  };
  const canCreate = () => name().trim() !== "" && runner() !== undefined;

  const create = () => {
    const host = runner();
    const trimmed = name().trim();
    if (!host || trimmed === "") return;
    const botID = store.createBot({
      name: trimmed,
      description: description().trim(),
      symbolName: "sparkles",
      accent: "indigo",
      runnerID: host.id,
      provider: provider(),
      model: model() || undefined,
      thinking: thinking() || undefined,
    });
    props.dismiss();
    props.onCreate(botID);
  };

  // The CLI's catalog, which comes with each snapshot, and the custom providers' saved models.
  const catalog = () => {
    track.roster();
    return withCustomModels(store.models, store.providers);
  };
  const models = () => providerModels(catalog(), provider());
  const levels = () => thinkingLevels(catalog(), provider(), model() || undefined);
  return (
    <Sheet
      title={L("New Bot")}
      subtitle={L("A bot runs on one Runner and uses that machine's credentials. Phones and tablets are not Runners.")}
      width={440}
      confirm={L("Create Bot")}
      confirmDisabled={!canCreate()}
      onConfirm={create}
      onCancel={props.dismiss}
    >
      <LabeledRow label={L("Name")}>
        <TextField value={name()} placeholder={L("Name")} autofocus onInput={setName} />
      </LabeledRow>
      <LabeledRow label={L("Description")} top>
        <TextArea
          value={description()}
          placeholder={L("What it does and how it should work")}
          rows={3}
          grows
          class="wrapping-field"
          onInput={setDescription}
          onKeyDown={(event) => {
            // Return confirms, as in a wrapping text field; Option or Shift with it starts a line.
            if (event.key === "Enter" && !event.isComposing && !event.altKey && !event.shiftKey) {
              event.preventDefault();
              if (canCreate()) create();
            }
          }}
        />
      </LabeledRow>
      <LabeledRow label={L("Runner")}>
        <PopUpButton
          options={runners().map((device) => ({ value: device.id, label: device.isThisDevice ? L("%@ (this computer)", device.name) : device.name }))}
          value={runner()?.id ?? ""}
          onChange={setRunnerID}
          class="fill"
        />
      </LabeledRow>
      <LabeledRow label={L("Provider")}>
        <PopUpButton
          options={kinds.map((kind) => ({ value: kind, label: `${providerName(kind, providers())} (${providerSubtitle(kind)})` }))}
          value={provider()}
          onChange={(kind) => {
            // A new provider starts on its default model and thinking level.
            setProvider(kind);
            setModel("");
            setThinking("");
          }}
          class="fill"
        />
      </LabeledRow>
      <LabeledRow label={L("Model")}>
        <PopUpButton
          options={[{ value: "", label: L("Default (%@)", models()[0]?.label ?? "") }, ...models().map((each) => ({ value: each.id, label: each.label }))]}
          value={model()}
          onChange={(id) => {
            setModel(id);
            // A level the new model does not take goes back to the default.
            if (!levels().some((level) => level.id === thinking())) setThinking("");
          }}
          class="fill"
        />
      </LabeledRow>
      {/* Only the levels this model takes; a model without any has no choice to make. */}
      <Show when={levels().length > 0}>
        <LabeledRow label={L("Thinking")}>
          <PopUpButton
            options={[{ value: "", label: L("Default") }, ...levels().map((level) => ({ value: level.id, label: level.label }))]}
            value={thinking()}
            onChange={setThinking}
            class="fill"
          />
        </LabeledRow>
      </Show>
      <div class={["sheet-note", { warning: note().warning }]}>{note().text}</div>
    </Sheet>
  );
}
