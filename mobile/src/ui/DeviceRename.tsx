import { useNativeState } from "@expo/ui";
import { AlertDialog, Column, Host, OutlinedTextField, Text as ComposeText, TextButton } from "@expo/ui/jetpack-compose";
import { fillMaxWidth } from "@expo/ui/jetpack-compose/modifiers";
import { useEffect, useRef, useState } from "react";
import { Alert, Platform } from "react-native";
import { engine } from "../core/engine";
import { deviceName, type Device } from "../core/model";
import { useStore } from "../core/store";
import { t } from "../i18n";
import { Row } from "./forms";

type Attempt = { active: boolean; submitted: boolean; id: string; name: string };

/** Native prompts retain their original account and selected Device, including across awaits. */
export function DeviceRename({ device }: { device: Device }) {
  const [dialog, setDialog] = useState<Attempt | null>(null);
  const authority = useStore(state => state);
  const text = useNativeState("");
  // Native observable writes are scheduled; submission never reads back a pending preset.
  const entered = useRef(device.name);
  const current = useRef<Attempt | null>(null);
  const selected = useRef(device.id);
  if (selected.current !== device.id) {
    if (current.current) current.current.active = false;
    selected.current = device.id;
  }
  useEffect(() => {
    const unsubscribe = useStore.subscribe((state, previous) => {
      if (state.identityId !== previous.identityId || state.deviceId !== previous.deviceId
        || state.relayUrl !== previous.relayUrl || state.paired !== previous.paired
        || !state.devices.some(item => item.id === selected.current)) {
        if (current.current) current.current.active = false;
        setDialog(null);
      }
    });
    return () => {
      unsubscribe();
      if (current.current) current.current.active = false;
    };
  }, []);

  function valid(attempt: Attempt) {
    const state = useStore.getState();
    return attempt.active && current.current === attempt && selected.current === attempt.id
      && state.paired && state.devices.some(item => item.id === attempt.id);
  }
  function cancel(attempt: Attempt) {
    if (attempt.submitted) return;
    attempt.active = false;
    setDialog(null);
  }
  async function submit(attempt: Attempt, value: string) {
    if (!valid(attempt) || attempt.submitted) return;
    attempt.submitted = true;
    setDialog(null);
    const name = value;
    if (name === attempt.name) return;
    try {
      await engine.setDeviceName(attempt.id, name);
    } catch (error: unknown) {
      if (valid(attempt)) Alert.alert(t("Could not rename Device"), error instanceof Error ? error.message : t("Could not rename Device"));
    }
  }
  function open() {
    if (current.current) current.current.active = false;
    const state = useStore.getState();
    if (selected.current !== device.id || state.identityId !== authority.identityId
      || state.deviceId !== authority.deviceId || state.relayUrl !== authority.relayUrl
      || state.paired !== authority.paired) return;
    const attempt: Attempt = { active: true, submitted: false, id: device.id, name: device.name };
    current.current = attempt;
    if (!valid(attempt)) return;
    if (Platform.OS === "ios") {
      Alert.prompt(t("Rename Device"), t("Every paired Device shows this name. Leave it blank to use the name the machine goes by."), [
        { text: t("Cancel"), style: "cancel", onPress: () => cancel(attempt) },
        { text: t("Rename"), onPress: value => { void submit(attempt, value ?? ""); } },
      ], "plain-text", device.name);
    } else {
      text.value = device.name;
      entered.current = device.name;
      setDialog(attempt);
    }
  }
  return <>
    <Row title={t("Rename…")} detail={deviceName(device)} onPress={open} />
    {Platform.OS === "android" && dialog?.active && dialog.id === device.id ? (
      <Host style={{ position: "absolute", top: 0, right: 0, bottom: 0, left: 0 }} pointerEvents="box-none">
        <AlertDialog onDismissRequest={() => cancel(dialog)}>
          <AlertDialog.Title><ComposeText>{t("Rename Device")}</ComposeText></AlertDialog.Title>
          <AlertDialog.Text>
            <Column verticalArrangement={{ spacedBy: 12 }} modifiers={[fillMaxWidth()]}>
            <ComposeText>{t("Every paired Device shows this name. Leave it blank to use the name the machine goes by.")}</ComposeText>
            <OutlinedTextField value={text} autoFocus singleLine onValueChange={value => { if (valid(dialog) && !dialog.submitted) entered.current = value; }} modifiers={[fillMaxWidth()]}>
              <OutlinedTextField.Placeholder><ComposeText>{device.machine_name || device.name}</ComposeText></OutlinedTextField.Placeholder>
            </OutlinedTextField>
            </Column>
          </AlertDialog.Text>
          <AlertDialog.ConfirmButton><TextButton onClick={() => { void submit(dialog, entered.current); }}><ComposeText>{t("Rename")}</ComposeText></TextButton></AlertDialog.ConfirmButton>
          <AlertDialog.DismissButton><TextButton onClick={() => cancel(dialog)}><ComposeText>{t("Cancel")}</ComposeText></TextButton></AlertDialog.DismissButton>
        </AlertDialog>
      </Host>
    ) : null}
  </>;
}
