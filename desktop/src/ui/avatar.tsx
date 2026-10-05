// Bot avatars are deterministic local SVGs keyed by bot ID, or the user's uploaded image.
// You and system keep their own discs; working dots and group layouts use the same circles.

import { blobatar } from "@beans/blobatar";
import { For, Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import { L } from "../l10n";
import type { Author, Bot } from "../model/models";
import { store } from "../model/store";
import { Icon } from "./icons";

export type AvatarContent =
  | { kind: "bot"; id: string }
  | { kind: "image"; url: string }
  | { kind: "you" }
  | { kind: "system" };

/** Uploaded image when fetched locally, otherwise the bot's deterministic SVG. */
export function botAvatar(bot: Bot): AvatarContent {
  const url = store.avatarURL(bot);
  if (url) return { kind: "image", url };
  return { kind: "bot", id: bot.id };
}

export function authorAvatar(author: Author): AvatarContent {
  if (author.kind === "you") return { kind: "you" };
  if (author.kind === "system") return { kind: "system" };
  const bot = store.bot(author.botID);
  return bot ? botAvatar(bot) : { kind: "system" };
}

export function sameAvatar(a: AvatarContent, b: AvatarContent): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "bot" && b.kind === "bot") return a.id === b.id;
  if (a.kind === "image" && b.kind === "image") return a.url === b.url;
  return true;
}

const svgURLs = new Map<string, string>();

function botSVGURL(id: string): string {
  let url = svgURLs.get(id);
  if (!url) {
    // SVG is generated in-package from the ID; encoding keeps markup out of the DOM.
    url = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(blobatar(id))}`;
    svgURLs.set(id, url);
  }
  return url;
}

/** Where the working dot sits: over the bottom-right edge of a circle `size` across. */
function presence(size: number) {
  const dot = Math.max(7, Math.round(size * 0.28));
  return { dot, x: size - dot + 1, y: size - dot + 1 };
}

function AvatarDisc(props: { content: AvatarContent; size: number; working?: boolean }) {
  const mask = (): JSX.CSSProperties | undefined => {
    if (!props.working) return undefined;
    const { dot, x, y } = presence(props.size);
    const cx = x + dot / 2;
    const cy = y + dot / 2;
    const cut = `radial-gradient(circle at ${cx}px ${cy}px, transparent ${dot / 2 + 2}px, #000 ${dot / 2 + 2.5}px)`;
    return { "-webkit-mask-image": cut, "mask-image": cut };
  };
  return (
    <span
      class={["avatar-disc", props.content.kind]}
      style={{
        width: `${props.size}px`,
        height: `${props.size}px`,
        ...mask(),
      }}
    >
      <Show when={props.content.kind === "image"}>
        <img src={props.content.kind === "image" ? props.content.url : undefined} alt="" draggable="false" />
      </Show>
      <Show when={props.content.kind === "bot"}>
        <img src={props.content.kind === "bot" ? botSVGURL(props.content.id) : undefined} alt="" draggable="false" />
      </Show>
      <Show when={props.content.kind === "you" || props.content.kind === "system"}>
        <Icon name={props.content.kind === "you" ? "person.fill" : "gearshape.fill"} size={Math.round(props.size * 0.5)} strokeWidth={props.size >= 40 ? 2 : 2.4} />
      </Show>
    </span>
  );
}

function PresenceDot(props: { size: number }) {
  const place = () => presence(props.size);
  return (
    <span
      class="presence-dot"
      style={{ width: `${place().dot}px`, height: `${place().dot}px`, left: `${place().x}px`, top: `${place().y}px` }}
    />
  );
}

/** One avatar. A click on one with `onClick` changes the bot's look. */
export function Avatar(props: {
  content: AvatarContent;
  size?: number;
  working?: boolean;
  class?: string;
  title?: string;
  onClick?: () => void;
}) {
  const size = () => props.size ?? 26;
  return (
    <span
      class={["avatar", props.class, { clickable: !!props.onClick }]}
      style={{ width: `${size()}px`, height: `${size()}px` }}
      title={props.onClick ? L("Change look") : props.title}
      role={props.onClick ? "button" : undefined}
      onClick={() => props.onClick?.()}
    >
      <AvatarDisc content={props.content} size={size()} working={props.working} />
      <Show when={props.working}>
        <PresenceDot size={size()} />
      </Show>
    </span>
  );
}

/** Group avatars packed into a fixed square: a row of overlapping circles would grow with the
 * member count and push the title along, and a constant slot keeps every row's text in line. */
export function AvatarCluster(props: { contents: AvatarContent[]; slot: number; working?: boolean }) {
  const boxes = () => {
    const slot = props.slot;
    const count = Math.min(props.contents.length, 4);
    if (count <= 1) return [{ x: 1, y: 1, size: slot - 2 }];
    const size = Math.round(slot * 0.6);
    const free = slot - size;
    switch (count) {
      case 2:
        return [
          { x: 0, y: 0, size },
          { x: free, y: free, size },
        ];
      case 3:
        return [
          { x: 0, y: 0, size },
          { x: free, y: 0, size },
          { x: free / 2, y: free, size },
        ];
      default:
        return [
          { x: 0, y: 0, size },
          { x: free, y: 0, size },
          { x: 0, y: free, size },
          { x: free, y: free, size },
        ];
    }
  };
  return (
    <span class="avatar-cluster" style={{ width: `${props.slot}px`, height: `${props.slot}px` }}>
      <For each={boxes()} keyed={false}>
        {(box, index) => (
          <span
            class={["cluster-cell", { front: index > 0 }]}
            style={{ left: `${box().x}px`, top: `${box().y}px`, width: `${box().size}px`, height: `${box().size}px` }}
          >
            <Show when={props.contents[index]}>
              {(content) => <Avatar content={content()} size={box().size} working={!!props.working && index === boxes().length - 1} />}
            </Show>
          </span>
        )}
      </For>
    </span>
  );
}

/** Overlapping avatars where there is room to spread out: up to three, later ones on top. */
export function AvatarStack(props: { bots: Bot[]; size: number; overlap: number }) {
  const shown = () => props.bots.slice(0, 3);
  return (
    <span class="avatar-stack" style={{ height: `${props.size}px`, width: `${props.size + Math.max(0, shown().length - 1) * (props.size - props.overlap)}px` }}>
      <For each={shown()}>
        {(bot, index) => (
          <span class={["stack-cell", { ringed: shown().length > 1 }]} style={{ left: `${index() * (props.size - props.overlap)}px` }}>
            <Avatar content={botAvatar(bot)} size={props.size} />
          </span>
        )}
      </For>
    </span>
  );
}
