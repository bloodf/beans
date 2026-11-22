// Settings › Secrets: the secrets a Runner keeps for its bots (#560), by name, never by value;
// adding one typed in here, deleting one, and revoking what the user allowed always. Each call
// goes to the picked Runner, which answers for the account's own Devices only.

import { createEffect, createSignal, For, Show } from "solid-js";
import { host } from "../../host";
import { L } from "../../l10n";
import type { Device } from "../../model/models";
import { grantDetail, grantTitle, secretSummary, validSecretName, type WireSecretGrant, type WireSecrets, type WireStoredSecret } from "../../model/secrets";
import { errorText, store } from "../../model/store";
import { TextField } from "../controls";
import { alert, presentSheet, Sheet } from "../overlay";
import { ActionRow, KeyValueRow, NoteRow, Section } from "../sections";
import { Footnote, PaneFrame, pickedDevice, Placeholder } from "./panes";

export function SecretsPane() {
  const device = pickedDevice;
  return (
    <PaneFrame pane="secrets">
      <Placeholder device={device()}>
        <Show when={device()}>{(runner) => <SecretsSettings device={runner()} />}</Show>
      </Placeholder>
      <Footnote
        text={L(
          "Bots use these secrets by name: a model only ever sees {{secret:NAME}}, and a command or plugin that would send one off the Runner asks you first. Values never leave the Runner, not even to this app.",
        )}
      />
    </PaneFrame>
  );
}

export function SecretsSettings(props: { device: Device }) {
  const [secrets, setSecrets] = createSignal<WireSecrets | null>(null);
  const [failure, setFailure] = createSignal<string | null>(null);
  let loads = 0;
  const load = async () => {
    const load = ++loads;
    try {
      const next = await store.listSecrets(props.device.id);
      if (load !== loads) return;
      setSecrets(next);
      setFailure(null);
    } catch (error) {
      if (load === loads) setFailure(errorText(error));
    }
  };
  createEffect(
    () => props.device.id,
    () => {
      setSecrets(null);
      void load();
    },
  );

  const remove = async (secret: WireStoredSecret) => {
    const answer = await alert({
      message: L("Delete “%@”?", secret.name),
      informative: L("Bots can no longer use it, and what was allowed always with it is revoked. This cannot be undone."),
      buttons: [{ title: L("Delete"), destructive: true }, { title: L("Cancel") }],
      style: "warning",
    });
    if (answer !== 0) return;
    try {
      await store.deleteSecret(props.device.id, secret.name);
      await load();
    } catch (error) {
      void alert({ message: L("Couldn't delete “%@”", secret.name), informative: errorText(error) });
    }
  };

  const revoke = async (grant: WireSecretGrant | null) => {
    try {
      if (grant === null) await store.revokeSecretGrants(props.device.id);
      // A grant is revoked for each secret it names, which ends it.
      else for (const name of grant.secrets) await store.revokeSecretGrants(props.device.id, name);
      await load();
    } catch (error) {
      void alert({ message: L("Couldn't revoke"), informative: errorText(error) });
    }
  };

  const add = () =>
    presentSecretEditor(props.device, async (name, value, note) => {
      await store.setSecret(props.device.id, name, value, note);
      await load();
    });

  return (
    <Show when={secrets()} fallback={<KeyValueRow label={failure() ?? L("Loading…")} value="" tint={failure() ? "var(--red)" : "var(--label-2)"} />}>
      {(current) => (
        <>
          <Section title={L("Secrets on %@", props.device.name)} style="heading">
            <For each={current().secrets} keyed={(secret) => secret.name}>
              {(secret) => (
                <ActionRow label={secret().name} monospaced value={secretSummary(secret())} tint="var(--label-2)" actionTitle={L("Delete…")} onAction={() => void remove(secret())} />
              )}
            </For>
            <Show when={current().secrets.length === 0}>
              <NoteRow text={L("No secrets yet. Bots keep the ones they set up here, or add one.")} tint="var(--label-2)" />
            </Show>
            <ActionRow label={L("Add a secret")} actionTitle={L("Add…")} onAction={add} />
          </Section>
          <Section title={L("Allowed Always")} style="heading">
            <For each={current().grants} keyed={(grant) => JSON.stringify(grant)}>
              {(grant) => <ActionRow label={grantTitle(grant())} monospaced value={grantDetail(grant())} tint="var(--label-2)" actionTitle={L("Revoke")} onAction={() => void revoke(grant())} />}
            </For>
            <Show when={current().grants.length === 0}>
              <NoteRow text={L("Nothing is allowed always. When a card asks to use a secret, Always allow keeps that exact use here.")} tint="var(--label-2)" />
            </Show>
            <Show when={current().grants.length > 1}>
              <ActionRow label={L("Revoke all")} actionTitle={L("Revoke All")} onAction={() => void revoke(null)} />
            </Show>
          </Section>
        </>
      )}
    </Show>
  );
}

/** A sheet to type a secret in: its name, its value (hidden, never shown again) and a note. */
function presentSecretEditor(device: Device, onSave: (name: string, value: string, note: string) => Promise<void>): void {
  presentSheet((dismiss) => {
    // The fields' text as typed, read by Save at once; the signal redraws the name's hint.
    const [name, setName] = createSignal("");
    let typedName = "";
    let value = "";
    let note = "";
    const [saving, setSaving] = createSignal(false);
    const save = async () => {
      const trimmed = typedName.trim();
      if (!validSecretName(trimmed) || value.trim() === "") {
        void host.beep();
        return;
      }
      setSaving(true);
      try {
        await onSave(trimmed, value, note);
        dismiss();
      } catch (error) {
        void alert({ message: L("Couldn't save “%@”", trimmed), informative: errorText(error) });
      } finally {
        setSaving(false);
      }
    };
    return (
      <Sheet
        title={L("Add a secret")}
        subtitle={L("Kept encrypted on %@ and never shown again. Bots use it by name.", device.name)}
        width={480}
        confirm={saving() ? L("Saving…") : L("Save")}
        onConfirm={() => void save()}
        onCancel={dismiss}
      >
        <div class="field-stack">
          <span class="sheet-caption">{L("Name").toUpperCase()}</span>
          <TextField value={name()} placeholder="STRIPE_API_KEY" monospaced autofocus label={L("Name")} onInput={(text) => { typedName = text; setName(text); }} />
          <Show when={name().trim() !== "" && !validSecretName(name().trim())}>
            <span class="sheet-caption" style={{ color: "var(--red)" }}>{L("Use letters, digits and _, starting with a letter.")}</span>
          </Show>
          <span class="sheet-caption">{L("Value").toUpperCase()}</span>
          <TextField value="" secure monospaced label={L("Value")} onInput={(text) => (value = text)} />
          <span class="sheet-caption">{L("Note").toUpperCase()}</span>
          <TextField value="" placeholder={L("What it is for")} label={L("Note")} onInput={(text) => (note = text)} />
        </div>
      </Sheet>
    );
  });
}
