// Bot look: deterministic ID-seeded avatar, or an uploaded image shared encrypted.
// Legacy symbol and accent remain on the wire, but no longer affect the rendered bot.

import { createSignal, Show } from "solid-js";
import { files, type FileInfo } from "../../host";
import { L } from "../../l10n";
import { store } from "../../model/store";
import { Avatar, botAvatar, type AvatarContent } from "../avatar";
import { Button } from "../controls";
import { alert, presentSheet, Sheet } from "../overlay";

/** Longest side of a stored profile image, as `Files.PrepareAvatar` makes it. */
const imageSide = 512;

type ImageChange = { kind: "keep" } | { kind: "remove" } | { kind: "set"; file: FileInfo };

export function presentBotLook(botID: string): void {
  presentSheet((dismiss) => <BotLookSheet botID={botID} dismiss={dismiss} />);
}

function BotLookSheet(props: { botID: string; dismiss: () => void }) {
  const [imageChange, setImageChange] = createSignal<ImageChange>({ kind: "keep" });

  /** Whether the saved look, with the pending change applied, has an image. */
  const hasImage = () => {
    const change = imageChange();
    if (change.kind === "keep") return store.bot(props.botID)?.avatar !== undefined;
    return change.kind === "set";
  };

  const preview = (): AvatarContent => {
    const change = imageChange();
    if (change.kind === "set") return { kind: "image", url: change.file.url };
    if (change.kind === "keep") {
      const bot = store.bot(props.botID);
      const saved = bot ? botAvatar(bot) : undefined;
      if (saved?.kind === "image") return saved;
    }
    return { kind: "bot", id: props.botID };
  };

  const chooseImage = async () => {
    const [picked] = await files.choose({ images: true, message: L("Choose an image for this bot.") });
    if (!picked) return;
    try {
      setImageChange({ kind: "set", file: await files.prepareAvatar(picked.path) });
    } catch {
      void alert({ message: L("That file could not be read as an image.") });
    }
  };

  const save = () => {
    const bot = store.bot(props.botID);
    if (bot) {
      const change = imageChange();
      if (change.kind === "remove") store.setBotAvatar(bot.id, null);
      else if (change.kind === "set") store.setBotAvatar(bot.id, change.file);
    }
    props.dismiss();
  };

  return (
    <Sheet
      title={L("Look")}
      subtitle={L("Each bot has its own look. Use an image of your own instead; paired Devices see it too.")}
      width={400}
      confirm={L("Save")}
      onConfirm={save}
      onCancel={props.dismiss}
      class="look-sheet"
    >
      <div class="look-preview">
        <Avatar content={preview()} size={72} />
      </div>
      <div class="look-heading">{L("Image").toUpperCase()}</div>
      <div class="look-image-buttons">
        <Button onClick={() => void chooseImage()}>{L("Choose Image…")}</Button>
        <Show when={hasImage()}>
          <Button onClick={() => setImageChange({ kind: "remove" })}>{L("Remove Image")}</Button>
        </Show>
      </div>
      <div class="look-caption">
        {L("Images are resized to %d px and shared encrypted, like an attachment.", imageSide)}
      </div>
    </Sheet>
  );
}
