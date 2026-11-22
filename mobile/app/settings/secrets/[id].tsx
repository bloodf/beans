// A Runner's secrets (#560), opened from its Device screen: the secrets its bots keep, by name
// and never by value; adding one typed in here; deleting one; and revoking what was allowed
// always. Every call goes to that Runner, which answers the account's own Devices only.

import { Stack, useLocalSearchParams } from "expo-router";
import { useCallback, useEffect, useState } from "react";
import { ActivityIndicator, Alert, ScrollView } from "react-native";
import { engine } from "../../../src/core/engine";
import { deviceName } from "../../../src/core/model";
import { validSecretName, type SecretGrant, type SecretsListing, type StoredSecret } from "../../../src/core/secrets";
import { useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { FieldRow, Row, Section } from "../../../src/ui/forms";
import { grantSubtitle, grantTitle, secretSummary } from "../../../src/ui/secretsText";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

export default function SecretsScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const device = useStore((s) => s.devices.find((d) => d.id === id));
  const [listing, setListing] = useState<SecretsListing | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [note, setNote] = useState("");
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    try {
      setListing(await engine.listSecrets(id));
      setFailure(null);
    } catch (error) {
      setFailure(message(error));
    }
  }, [id]);
  useEffect(() => void load(), [load]);

  const save = async () => {
    const trimmed = name.trim();
    if (!validSecretName(trimmed) || !value.trim()) {
      Alert.alert(t("Check the name and value"), t("Use letters, digits and _ for the name, starting with a letter, and type the value."));
      return;
    }
    setSaving(true);
    try {
      await engine.setSecret(id, trimmed, value, note.trim());
      setName("");
      setValue("");
      setNote("");
      await load();
    } catch (error) {
      Alert.alert(t("Couldn’t save {name}", { name: trimmed }), message(error));
    } finally {
      setSaving(false);
    }
  };

  const remove = (secret: StoredSecret) =>
    Alert.alert(t("Delete {name}?", { name: secret.name }), t("Bots can no longer use it, and what was allowed always with it is revoked."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Delete"),
        style: "destructive",
        onPress: () => void engine.deleteSecret(id, secret.name).then(load, (error: unknown) => Alert.alert(t("Couldn’t delete {name}", { name: secret.name }), message(error))),
      },
    ]);

  const revoke = (grant: SecretGrant) =>
    Alert.alert(t("Revoke this?"), grantTitle(grant), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Revoke"),
        style: "destructive",
        onPress: () =>
          void (async () => {
            for (const secret of grant.secrets) await engine.revokeSecretGrants(id, secret);
          })().then(load, (error: unknown) => Alert.alert(t("Couldn’t revoke"), message(error))),
      },
    ]);

  const title = device ? t("Secrets on {name}", { name: deviceName(device) }) : t("Secrets");
  return (
    <>
      <Stack.Screen options={{ title: t("Secrets") }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardShouldPersistTaps="handled">
        {!listing ? (
          <Section>{failure ? <Row title={failure} destructive /> : <Row title={t("Loading…")} accessory={<ActivityIndicator />} />}</Section>
        ) : (
          <>
            <Section title={title} footer={t("Bots use these by name: a model only ever sees {{secret:NAME}}. Values never leave the Runner, not even to this phone. Tap one to delete it.")}>
              {listing.secrets.length === 0 ? <Row title={t("No secrets yet")} /> : listing.secrets.map((secret) => <Row key={secret.name} title={secret.name} subtitle={secretSummary(secret)} subtitleLines={2} onPress={() => remove(secret)} chevron />)}
            </Section>
            <Section title={t("Add a secret")} footer={t("Kept encrypted on the Runner and never shown again.")}>
              <FieldRow label={t("Name")} value={name} onChangeText={setName} placeholder="STRIPE_API_KEY" autoCapitalize="characters" autoCorrect={false} />
              <FieldRow label={t("Value")} value={value} onChangeText={setValue} secureTextEntry autoCapitalize="none" autoCorrect={false} />
              <FieldRow label={t("Note")} value={note} onChangeText={setNote} placeholder={t("What it is for")} />
              <Row title={saving ? t("Saving…") : t("Save Secret")} icon="key.fill" onPress={saving ? undefined : () => void save()} />
            </Section>
            <Section title={t("Allowed always")} footer={t("When a card asks to use a secret, Always allow keeps that exact use here. Tap one to revoke it.")}>
              {listing.grants.length === 0 ? <Row title={t("Nothing is allowed always")} /> : listing.grants.map((grant, index) => <Row key={index} title={grantTitle(grant)} subtitle={grantSubtitle(grant)} onPress={() => revoke(grant)} destructive />)}
            </Section>
          </>
        )}
      </ScrollView>
    </>
  );
}
