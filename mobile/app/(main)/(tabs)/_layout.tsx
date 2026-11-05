import { usePathname, useRouter } from "expo-router";
import { NativeTabs } from "expo-router/unstable-native-tabs";
import { useEffect } from "react";
import { t, useLanguage } from "../../../src/i18n";
import { useWide } from "../../../src/ui/layout";
import { usePalette } from "../../../src/ui/theme";

export const unstable_settings = { initialRouteName: "(chats)" };

export default function TabsLayout() {
  useLanguage();
  const p = usePalette();
  const wide = useWide();
  const router = useRouter();
  const pathname = usePathname();

  useEffect(() => {
    // Devices remain available in Settings; the wide pane belongs to the selected chat.
    if (wide && (pathname.startsWith("/devices") || pathname === "/search")) router.navigate("/");
  }, [wide, pathname, router]);

  return (
    <NativeTabs hidden={wide} tintColor={p.tint}>
      <NativeTabs.Trigger name="(chats)">
        <NativeTabs.Trigger.Icon sf={{ default: "bubble.left.and.bubble.right", selected: "bubble.left.and.bubble.right.fill" }} md="forum" />
        <NativeTabs.Trigger.Label>{t("Bots")}</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="devices">
        <NativeTabs.Trigger.Icon sf="laptopcomputer.and.iphone" md="devices" />
        <NativeTabs.Trigger.Label>{t("Devices")}</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="search" role="search">
        <NativeTabs.Trigger.Icon sf="magnifyingglass" md="search" />
        <NativeTabs.Trigger.Label>{t("Search")}</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
    </NativeTabs>
  );
}
