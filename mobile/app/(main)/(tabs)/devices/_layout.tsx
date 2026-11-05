import { Stack } from "expo-router";
import { Platform } from "react-native";
import { t, useLanguage } from "../../../../src/i18n";
import { useStackScreenOptions } from "../../../../src/ui/navigation";
import { usePalette } from "../../../../src/ui/theme";

export default function DevicesLayout() {
  useLanguage();
  const p = usePalette();
  const screenOptions = useStackScreenOptions();
  return (
    <Stack screenOptions={{ ...screenOptions, headerTransparent: Platform.OS === "ios", headerStyle: { backgroundColor: p.groupedBackground }, contentStyle: { backgroundColor: p.groupedBackground } }}>
      <Stack.Screen name="index" options={{ title: t("Devices"), headerLargeTitle: Platform.OS === "ios" }} />
      <Stack.Screen name="device/[id]" />
    </Stack>
  );
}
