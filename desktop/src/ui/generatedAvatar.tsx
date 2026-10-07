import { botAvatarGeometry, resolveBotAppearance, type AvatarGeometry, type AvatarFrame } from "@beans/blobatar";
import { avatarPath } from "@beans/blobatar/frame";
import { createEffect, For, onSettled, Show } from "solid-js";
import { L } from "../l10n";
import { lookProblem, type BotLook, type BotAvatarState } from "../model/botLook";
import { avatarClock } from "./avatarClock";
import { AvatarMotion } from "./avatarMotion";
import { watchAvatarVisibility } from "./avatarVisibility";
import { Icon } from "./icons";

const endpoints = new Map<string, AvatarGeometry>();
const petalSlots = [0, 1, 2, 3, 4, 5, 6, 7, 8];
const eyeSlots = [0, 1] as const;
function endpoint(id: string, look: BotLook | undefined, state: BotAvatarState) {
  const key = `${id}\0${JSON.stringify(resolveBotAppearance(id, look, state))}`;
  let geometry = endpoints.get(key);
  if (!geometry) {
    geometry = botAvatarGeometry(id, look, state);
    if (endpoints.size >= 256) endpoints.delete(endpoints.keys().next().value!);
    endpoints.set(key, geometry);
  }
  return geometry;
}

export function GeneratedAvatar(props: { id: string; look?: BotLook; state?: BotAvatarState }) {
  let svg: SVGSVGElement | undefined;
  let background: SVGPathElement | undefined, core: SVGPathElement | undefined, extra: SVGPathElement | undefined;
  let body: SVGGElement | undefined;
  const petals: SVGCircleElement[] = [], eyes: SVGPathElement[] = [];
  let motion: AvatarMotion | undefined, target: AvatarGeometry | undefined, currentID: string | undefined;
  let visible = false, reducedMotion = false, stop: (() => void) | undefined;
  let mounted = false;
  let painted: AvatarFrame | undefined;
  const supported = () => !lookProblem(props.look);
  const animate = () => visible && !reducedMotion && !!target?.motion;

  const paint = (frame: AvatarFrame) => {
    if (!svg || !background || !core || !extra || !body) return;
    if (painted?.background.path !== frame.background.path) background.setAttribute("d", avatarPath(frame.background.path));
    if (painted?.background.fill !== frame.background.fill) background.setAttribute("fill", frame.background.fill);
    if (painted?.background.opacity !== frame.background.opacity) background.setAttribute("opacity", String(frame.background.opacity));
    body.setAttribute("transform", `matrix(${frame.body.join(" ")})`);
    if (painted?.head !== frame.head) body.setAttribute("fill", frame.head);
    if (painted?.core !== frame.core) core.setAttribute("d", avatarPath(frame.core));
    if (painted?.extra !== frame.extra) extra.setAttribute("d", avatarPath(frame.extra));
    for (let index = 0; painted?.petals !== frame.petals && index < petals.length; index++) {
      const circle = frame.petals[index]!;
      petals[index]!.setAttribute("cx", String(circle.cx));
      petals[index]!.setAttribute("cy", String(circle.cy));
      petals[index]!.setAttribute("r", String(circle.r));
    }
    for (const index of eyeSlots) {
      if (painted?.eyes[index] !== frame.eyes[index]) eyes[index]!.setAttribute("d", avatarPath(frame.eyes[index]));
      eyes[index]!.setAttribute("transform", `matrix(${frame.eyeMatrices[index].join(" ")})`);
      if (painted?.eye !== frame.eye) eyes[index]!.setAttribute("fill", frame.eye);
    }
    painted = frame;
  };
  const frame = (time: number) => { if (motion) paint(motion.frame(time, animate())); };
  const synchronize = () => {
    if (!mounted) return;
    if (animate() && supported()) {
      frame(performance.now());
      stop ??= avatarClock.subscribe(frame);
    } else {
      stop?.(); stop = undefined;
      frame(performance.now());
    }
  };
  const update = () => {
    if (!supported()) { stop?.(); stop = undefined; return; }
    const next = endpoint(props.id, props.look, props.state ?? "idle");
    const sameBot = currentID === props.id;
    target = next;
    if (!motion || !sameBot) motion = new AvatarMotion(next);
    else motion.retarget(next, performance.now(), animate());
    currentID = props.id;
    synchronize();
  };
  createEffect(() => ({ id: props.id, look: props.look, state: props.state }), update);
  onSettled(() => {
    mounted = true;
    update();
    const unwatch = svg ? watchAvatarVisibility(svg, (nextVisible, reduce) => {
      visible = nextVisible; reducedMotion = reduce; synchronize();
    }) : undefined;
    return () => { mounted = false; stop?.(); unwatch?.(); };
  });

  return (
    <>
      <svg ref={(element) => (svg = element)} viewBox="0 0 100 100" class="generated-avatar" aria-hidden="true" style={{ display: supported() ? undefined : "none" }}>
        <path ref={(element) => (background = element)} />
        <g ref={(element) => (body = element)}>
          <For each={petalSlots}>{(index) => <circle ref={(element) => (petals[index] = element)} />}</For>
          <path ref={(element) => (core = element)} />
          <path ref={(element) => (extra = element)} />
        </g>
        <For each={eyeSlots}>{(index) => <path ref={(element) => (eyes[index] = element)} />}</For>
      </svg>
      <Show when={!supported()}><span role="img" aria-label={L("Unsupported avatar appearance. Update the app to edit it.")}><Icon name="exclamationmark.circle.fill" size={16} /></span></Show>
    </>
  );
}
