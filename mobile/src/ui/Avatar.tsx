import type { BotAvatarState, BotLook } from "@beans/blobatar";
import { Image } from "expo-image";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { StyleSheet, Text, View, useWindowDimensions, type ColorValue, type StyleProp, type ViewStyle } from "react-native";
import Animated, { cancelAnimation, Easing, useAnimatedStyle, useSharedValue, withRepeat, withTiming } from "react-native-reanimated";
import { resolveBotState, resolveChatBotState } from "../core/activity";
import { engine } from "../core/engine";
import type { Bot } from "../core/model";
import { useStore } from "../core/store";
import { AvatarFigure } from "./AvatarFigure";
import { avatarGeometry } from "./avatarGeometry";
import { useReduceMotion, useScreenFocused } from "./avatarPrefs";
import { avatarIntersectsViewport, subscribeAvatarScroll, useAvatarRowVisible } from "./avatarVisibility";
import { Symbol } from "./Symbol";
import { usePalette } from "./theme";

export function useBotAvatarUri(bot: Bot | undefined): string | undefined {
  const avatar = bot?.avatar;
  const uri = useStore((s) => (avatar ? s.files[avatar.id] : undefined));
  useEffect(() => {
    if (avatar && !uri) void engine.fetchFile(avatar);
  }, [avatar, uri]);
  return avatar ? uri : undefined;
}

// Native measurements cover scrollable forms; FlashList's explicit row viewability also stops
// draw-distance cells. Measurements publish only visibility changes, not animation frames.
function useSurfaceVisibility() {
  const ref = useRef<View>(null);
  const [onScreen, setOnScreen] = useState(false);
  const rowVisible = useAvatarRowVisible();
  const appActive = useStore((s) => s.appActive);
  const focused = useScreenFocused();
  const reduceMotion = useReduceMotion();
  const { width, height } = useWindowDimensions();
  const alive = useRef(false);
  const measurement = useRef(0);
  const check = useCallback(() => {
    const sequence = ++measurement.current;
    ref.current?.measureInWindow((x, y, w, h) => {
      if (alive.current && sequence === measurement.current) setOnScreen(avatarIntersectsViewport(x, y, w, h, width, height));
    });
  }, [width, height]);
  useEffect(() => {
    alive.current = true;
    check();
    const unsubscribe = subscribeAvatarScroll(check);
    return () => { alive.current = false; ++measurement.current; unsubscribe(); };
  }, [check]);
  useEffect(check, [check, appActive, focused, rowVisible]);
  return { ref, check, visible: onScreen && rowVisible, appActive, focused, reduceMotion };
}

// Only generated portraits mount the numeric engine. Photo-circle cropping is independent of
// saved look; missing photo bytes use that customized generated fallback.
export function AvatarDisc({ name, uri, size, look, state = "idle", working = false, service, onVisibilityChange, style }: { name: string; uri?: string; size: number; look?: BotLook; state?: BotAvatarState; working?: boolean; service?: string; onVisibilityChange?: (visible: boolean) => void; style?: StyleProp<ViewStyle> }) {
  const p = usePalette();
  const surface = useSurfaceVisibility();
  const geometry = useMemo(() => uri ? null : avatarGeometry(name, look, state), [name, uri, look, state]);
  useEffect(() => { onVisibilityChange?.(surface.visible); }, [surface.visible, onVisibilityChange]);
  return (
    <View ref={surface.ref} collapsable={false} onLayout={surface.check} style={[styles.disc, { width: size, height: size }, style]}>
      {uri ? (
        <View style={{ width: size, height: size, borderRadius: size / 2, overflow: "hidden", backgroundColor: p.fill }}>
          <Image source={{ uri }} contentFit="cover" style={{ width: size, height: size }} />
        </View>
      ) : geometry ? <AvatarFigure key={name} geometry={geometry} size={size} visibility={{ ...surface, motion: geometry.motion }} /> : null}
      {service ? <ServiceBadge service={service} size={size} ring={p.background} /> : null}
      {working ? <PresenceDot size={size} ring={p.background} top={!!service} moving={!!geometry?.motion && surface.visible && surface.appActive && surface.focused && !surface.reduceMotion} /> : null}
    </View>
  );
}


export function useBotAvatarState(botId: string | undefined, chatId?: string): BotAvatarState {
  return useStore((s) => (!botId ? "idle" : chatId ? resolveChatBotState(s, chatId, botId) : resolveBotState(s, botId)));
}

export function BotAvatar({ bot, chatId, size = 40, working = false, service, style }: { bot: Bot | undefined; chatId?: string; size?: number; working?: boolean; service?: string; style?: StyleProp<ViewStyle> }) {
  const uri = useBotAvatarUri(bot);
  const state = useBotAvatarState(bot?.id, chatId);
  return <AvatarDisc name={bot?.id ?? ""} uri={uri} size={size} look={bot?.look} state={state} working={working} service={service} style={style} />;
}

export function YouAvatar({ size = 40 }: { size?: number }) {
  const p = usePalette();
  return <View style={[styles.disc, { width: size, height: size, borderRadius: size / 2, backgroundColor: p.fill }]}><Symbol name="person.fill" size={size * 0.5} color={p.secondaryLabel} /></View>;
}

export function PresenceDot({ size, ring, moving, top = false }: { size: number; ring: ColorValue; moving: boolean; top?: boolean }) {
  const p = usePalette();
  const scale = useSharedValue(1);
  useEffect(() => {
    cancelAnimation(scale);
    scale.value = 1;
    if (moving) scale.value = withRepeat(withTiming(0.8, { duration: 1200, easing: Easing.inOut(Easing.ease) }), -1, true);
    return () => cancelAnimation(scale);
  }, [moving, scale]);
  const animated = useAnimatedStyle(() => ({ transform: [{ scale: scale.value }] }));
  const dot = Math.max(7, Math.round(size * 0.27));
  const ringWidth = Math.max(1.5, dot * 0.22);
  return (
    <Animated.View style={[styles.presence, animated, { width: dot + ringWidth * 2, height: dot + ringWidth * 2, borderRadius: (dot + ringWidth * 2) / 2, backgroundColor: ring, right: -ringWidth * 0.6, ...(top ? { top: -ringWidth * 0.6 } : { bottom: -ringWidth * 0.6 }) }]}>
      <View style={{ width: dot, height: dot, borderRadius: dot / 2, backgroundColor: p.green }} />
    </Animated.View>
  );
}

export function AvatarCluster({ bots, chatId, size = 40, working = false, service }: { bots: Bot[]; chatId?: string; size?: number; working?: boolean; service?: string }) {
  const p = usePalette();
  const surface = useSurfaceVisibility();
  if (bots.length <= 1) return <BotAvatar bot={bots[0]} chatId={chatId} size={size} working={working} service={service} />;
  const small = size * 0.66;
  return (
    <View ref={surface.ref} collapsable={false} onLayout={surface.check} style={{ width: size, height: size }}>
      <BotAvatar bot={bots[1]} chatId={chatId} size={small} style={{ position: "absolute", right: 0, top: 0 }} />
      <View style={{ position: "absolute", left: 0, bottom: 0, width: small + 4, height: small + 4, borderRadius: (small + 4) / 2, backgroundColor: p.background, alignItems: "center", justifyContent: "center" }}>
        <BotAvatar bot={bots[0]} chatId={chatId} size={small} />
      </View>
      {working ? <PresenceDot size={size} ring={p.background} moving={surface.visible && surface.appActive && surface.focused && !surface.reduceMotion && bots.some((b) => !b.avatar)} /> : null}
    </View>
  );
}

const styles = StyleSheet.create({
  disc: { alignItems: "center", justifyContent: "center" },
  presence: { position: "absolute", alignItems: "center", justifyContent: "center" },
  badge: { position: "absolute", alignItems: "center", justifyContent: "center" },
  eye: { position: "absolute", backgroundColor: "#FFFFFF" },
  shapeGrid: { flexDirection: "row", flexWrap: "wrap", gap: 10, padding: 14 },
  shapeCell: { width: 48, height: 44, borderRadius: 10, borderWidth: 2, alignItems: "center", justifyContent: "center" },
});

const SERVICE_MARKS: Record<string, number> = {
  telegram: require("../../assets/services/telegram.png"),
  slack: require("../../assets/services/slack.png"),
};

/// A channel conversation's service mark over the avatar's bottom-right edge, ringed in the
/// surface color.
function ServiceBadge({ service, size, ring }: { service: string; size: number; ring: ColorValue }) {
  const mark = SERVICE_MARKS[service];
  if (!mark) return null;
  const badge = Math.round(size * 0.4);
  const ringWidth = Math.max(1.5, badge * 0.1);
  const outer = badge + ringWidth * 2;
  return (
    <View style={[styles.badge, { width: outer, height: outer, borderRadius: outer / 2, backgroundColor: ring, right: -ringWidth, bottom: -ringWidth }]}>
      <Image source={mark} style={{ width: badge, height: badge }} />
    </View>
  );
}


export function ContactAvatar({ name, size = 40 }: { name: string; size?: number }) {
  const initial = Array.from(name.trim())[0]?.toUpperCase() ?? "?";
  return (
    <View style={[styles.disc, { width: size, height: size, borderRadius: size / 2 }]}>
      <Text style={{ color: "#FFFFFF", fontSize: Math.round(size * 0.46), fontWeight: "600" }} allowFontScaling={false}>
        {initial}
      </Text>
    </View>
  );
}
