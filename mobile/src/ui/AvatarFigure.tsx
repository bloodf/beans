import { avatarFrame, avatarPath, type AvatarFrame, type AvatarGeometry, type AvatarMatrix } from "@beans/blobatar/frame";
import { useCallback, useEffect, useMemo, useRef, type ComponentType } from "react";
import Svg, { Circle, G, Path, type GProps } from "react-native-svg";
import Animated, { createAnimatedPropAdapter, processColor, useAnimatedProps, useAnimatedReaction, useFrameCallback, useSharedValue, type SharedValue } from "react-native-reanimated";
import { createAvatarClock, type AvatarClock, type AvatarVisibility } from "./avatarClock";
import { displayedGeometry } from "./avatarMotion";

const AnimatedGroup = Animated.createAnimatedComponent(G as unknown as ComponentType<GProps & { matrix?: AvatarMatrix }>);
const AnimatedPath = Animated.createAnimatedComponent(Path);
const AnimatedCircle = Animated.createAnimatedComponent(Circle);

// Native RNSVG's fill is a brush, not a JS string. The worklet writes the native ColorStruct
// directly (Fabric and both native platforms accept {type:0,payload:<processed color>}).
const brushAdapter = createAnimatedPropAdapter((props) => {
  if (typeof props.fill === "string") props.fill = { type: 0, payload: processColor(props.fill) };
}, ["fill"]);

interface TargetRequest { geometry: AvatarGeometry; active: boolean }

// Geometry/endpoints are computed on JS only when the bot/look/state changes. Every intermediate
// contour, eye, matrix and color comes from shared numeric worklets, never per-frame React state.
export function AvatarFigure({ geometry, size, visibility }: { geometry: AvatarGeometry; size: number; visibility: AvatarVisibility }) {
  const initial = useMemo(() => avatarFrame(geometry, 0, 0), [geometry]);
  const request = useSharedValue<TargetRequest>({ geometry, active: false });
  const from = useSharedValue(geometry);
  const toward = useSharedValue(geometry);
  const shown = useSharedValue(geometry);
  const frame = useSharedValue<AvatarFrame>(initial);
  const started = useSharedValue(-1);
  const morphing = useSharedValue(false);
  const enabled = useSharedValue(false);

  const tick = useCallback((info: { timestamp: number }) => {
    "worklet";
    if (!enabled.value) return;
    if (morphing.value) {
      if (started.value < 0) started.value = info.timestamp;
      shown.value = displayedGeometry(from.value, toward.value, info.timestamp - started.value);
      // The longest upstream morph takes 400ms. No interpolation allocations after it settles.
      if (info.timestamp - started.value >= 400) {
        shown.value = toward.value;
        morphing.value = false;
      }
    }
    frame.value = avatarFrame(shown.value, info.timestamp, 1);
  }, [enabled, morphing, started, shown, from, toward, frame]);
  const callback = useFrameCallback(tick, false);
  const clock = useRef<AvatarClock | null>(null);

  useAnimatedReaction(() => request.value, (next) => {
    "worklet";
    enabled.value = next.active;
    if (!next.active) {
      from.value = next.geometry;
      toward.value = next.geometry;
      shown.value = next.geometry;
      morphing.value = false;
      frame.value = avatarFrame(next.geometry, 0, 0);
      return;
    }
    // shown is the last painted geometry, not a theoretical future point. An interrupted
    // retarget starts there; several target writes before a paint cannot overwrite it.
    from.value = shown.value;
    toward.value = next.geometry;
    started.value = -1;
    morphing.value = true;
  }, [request, enabled, from, toward, shown, morphing, frame, started]);

  useEffect(() => {
    const owner = createAvatarClock((active) => {
      if (callback.callbackId !== -1) callback.setActive(active);
    });
    clock.current = owner;
    return () => { owner.dispose(); clock.current = null; };
  }, [callback]);

  const { appActive, focused, reduceMotion, visible, motion } = visibility;
  useEffect(() => {
    const active = visible && appActive && focused && !reduceMotion && motion;
    request.value = { geometry, active };
    clock.current?.update({ visible, appActive, focused, reduceMotion, motion });
  }, [geometry, visible, appActive, focused, reduceMotion, motion, request]);

  const background = useAnimatedProps(() => ({ d: avatarPath(frame.value.background.path), fill: frame.value.background.fill, opacity: frame.value.background.opacity }), [], brushAdapter);
  const body = useAnimatedProps(() => ({ matrix: frame.value.body }));
  const extra = useAnimatedProps(() => ({ d: avatarPath(frame.value.extra), fill: frame.value.head }), [], brushAdapter);
  const core = useAnimatedProps(() => ({ d: avatarPath(frame.value.core), fill: frame.value.head }), [], brushAdapter);

  return (
    <Svg width={size} height={size} viewBox="0 0 100 100" accessible={false} pointerEvents="none">
      <AnimatedPath animatedProps={background} />
      <AnimatedGroup animatedProps={body}>
        {geometry.petals.map((_, index) => <Petal key={index} index={index} frame={frame} />)}
        <AnimatedPath animatedProps={extra} />
        <AnimatedPath animatedProps={core} />
      </AnimatedGroup>
      <Eye index={0} frame={frame} />
      <Eye index={1} frame={frame} />
    </Svg>
  );
}

function Petal({ index, frame }: { index: number; frame: SharedValue<AvatarFrame> }) {
  const props = useAnimatedProps(() => {
    const circle = frame.value.petals[index];
    return { cx: circle.cx, cy: circle.cy, r: circle.r, fill: frame.value.head };
  }, [index], brushAdapter);
  return <AnimatedCircle animatedProps={props} />;
}

function Eye({ index, frame }: { index: 0 | 1; frame: SharedValue<AvatarFrame> }) {
  const matrix = useAnimatedProps(() => ({ matrix: frame.value.eyeMatrices[index] }), [index]);
  const path = useAnimatedProps(() => ({ d: avatarPath(frame.value.eyes[index]), fill: frame.value.eye }), [index], brushAdapter);
  // eyeMatrices already contain body; nesting inside body would transform the eyes twice.
  return <AnimatedGroup animatedProps={matrix}><AnimatedPath animatedProps={path} /></AnimatedGroup>;
}
