import { useRouter } from "expo-router";
import { StyleSheet, View } from "react-native";
import { engine } from "../core/engine";
import { deviceName, isRunner } from "../core/model";
import { deviceIsOnline, useStore } from "../core/store";
import { t, useLanguage } from "../i18n";
import { deviceSymbol } from "./devices";
import { lastSeen } from "./format";
import { Row, Section } from "./forms";
import { Symbol } from "./Symbol";
import { usePalette } from "./theme";

/** The same paired Device roster in the phone tab and wide-window Settings sheet. */
export function DeviceRows({ base }: { base: "/devices" | "/settings" }) {
  useLanguage();
  const router = useRouter();
  const p = usePalette();
  const devices = useStore(s => s.devices);
  const seen = useStore(s => s.device_seen);
  const thisId = engine.deviceId;
  const sorted = [...devices].sort((a, b) =>
    a.id === thisId ? -1 : b.id === thisId ? 1 : Number(!!a.unknown) - Number(!!b.unknown) || a.name.localeCompare(b.name),
  );
  return (
    <Section title={t("Devices")} footer={t("Desktop Devices are Runners: they run bots with your account's provider credentials. Phones and tablets read and write chats.")}>
      {sorted.map(device => {
        const online = device.id === thisId || deviceIsOnline(device.id);
        return (
          <Row
            key={device.id}
            title={device.id === thisId ? t("{name} (this phone)", { name: device.name }) : deviceName(device)}
            onPress={() => router.push(`${base}/device/${device.id}`)}
            chevron
            subtitle={[device.model, isRunner(device) ? t("Runner") : t("Device"), online ? t("Online") : lastSeen(seen[device.id])].filter(Boolean).join(" · ")}
            leading={
              <View style={styles.icon}>
                <Symbol name={deviceSymbol(device.os, device.model)} size={22} color={p.label} />
                <View style={[styles.dot, { backgroundColor: online ? p.green : p.tertiaryLabel, borderColor: p.cell }]} />
              </View>
            }
          />
        );
      })}
    </Section>
  );
}

const styles = StyleSheet.create({
  icon: { width: 32, alignItems: "center" },
  dot: { position: "absolute", right: 0, bottom: -2, width: 10, height: 10, borderRadius: 5, borderWidth: 2 },
});
