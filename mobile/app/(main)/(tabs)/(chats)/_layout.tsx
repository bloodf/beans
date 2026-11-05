import { Stack } from "expo-router";
import { Platform } from "react-native";
import { t, useLanguage } from "../../../../src/i18n";
import { useStackScreenOptions } from "../../../../src/ui/navigation";

export default function ChatsLayout() {
  useLanguage();
  const screenOptions = useStackScreenOptions();
  return (
    <Stack screenOptions={screenOptions}>
      <Stack.Screen name="index" options={{ title: t("Chats"), headerTitle: "", headerLargeTitle: false, headerShadowVisible: false, headerTransparent: Platform.OS === "ios" }} />
    </Stack>
  );
}
