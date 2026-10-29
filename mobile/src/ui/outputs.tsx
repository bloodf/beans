// A bot's published output in the transcript: a card of its own without a bubble, and a compact
// row when several of one bot's outputs sit back to back.

import { Image } from "expo-image";
import { useRouter } from "expo-router";
import { useEffect } from "react";
import { Platform, Pressable, StyleSheet, Text, View } from "react-native";
import { engine } from "../core/engine";
import {
  evidenceStatus,
  evidenceTitle,
  fileSize,
  outputKind,
  outputSymbol,
  outputTypeName,
  type Attachment,
  type Message,
  type Output,
  type OutputKind,
} from "../core/model";
import { useStore } from "../core/store";
import { t, useLanguage } from "../i18n";
import { Symbol } from "./Symbol";
import { accentColor, Font, usePalette, type Palette } from "./theme";

export function outputAttachment(message: Message): Attachment | undefined {
  return message.body.kind === "text" ? message.body.attachments?.[0] : undefined;
}

/// A card's widest, and the image preview's tallest and shortest.
const CARD_WIDTH = 320;
const PREVIEW_MAX = 200;
const PREVIEW_MIN = 96;
/// The icon's box in a run's row, where an image shows its own picture.
const STACK_ICON = 24;

const KIND_ACCENTS: Partial<Record<OutputKind, string>> = {
  link: "blue",
  image: "blue",
  video: "purple",
  audio: "pink",
  pdf: "red",
  code: "orange",
  data: "teal",
  table: "green",
  text: "indigo",
};

/// The color of an output's icon, by what it is.
function outputTint(output: Output, p: Palette) {
  const accent = KIND_ACCENTS[outputKind(output)];
  return accent ? accentColor(accent, p.dark) : p.secondaryLabel;
}

/// A published output in the transcript, standing without a bubble or a time: an image's preview
/// (the top of a tall one, as a page's screenshot starts at its top), the file's icon, name, type
/// and size, and what the bot checked. A tap opens the output's screen, or fetches again a file
/// that has not landed yet. A run of one bot's outputs joins into one list (`stack` says whether
/// the cards above and below are in it), a compact row each: the icon (an image's own picture),
/// name, type and size, and the check's symbol; the check's words are on the output's screen.
export function OutputCard({ message, maxWidth, stack }: { message: Message; maxWidth: number; stack?: { above: boolean; below: boolean } }) {
  useLanguage();
  const p = usePalette();
  const router = useRouter();
  const output = message.output!;
  const attachment = outputAttachment(message);
  const uri = useStore((s) => (attachment ? s.files[attachment.id] : undefined));
  useEffect(() => {
    if (attachment && !uri) void engine.fetchFile(attachment);
  }, [uri, attachment]);
  const width = Math.min(CARD_WIDTH, maxWidth);
  const kind = outputKind(output);
  const tint = outputTint(output, p);
  const detail = [outputTypeName(output), attachment ? fileSize(attachment.size) : ""].filter(Boolean).join(" · ");
  const stacked = !!stack && (stack.above || stack.below);
  let preview: number | undefined;
  if (kind === "image" && attachment && !stacked) {
    const full = (width * Math.max(attachment.height ?? 3, 1)) / Math.max(attachment.width ?? 4, 1);
    preview = Math.min(Math.max(full, PREVIEW_MIN), PREVIEW_MAX);
  }
  const evidence = output.evidence;
  const checkTint = evidence?.status === "passed" ? p.green : evidence?.status === "failed" ? p.red : p.secondaryLabel;
  const checkSymbol = evidence?.status === "passed" ? "checkmark.circle.fill" : evidence?.status === "failed" ? "xmark.circle.fill" : "circle.dashed";
  const summary = evidence?.summary.trim() ?? "";
  const spoken = [output.name, detail, evidence ? `${evidenceTitle(evidence)} · ${evidenceStatus(evidence)}: ${summary}` : ""].filter(Boolean).join(", ");
  const open = () => {
    if (attachment && !uri) {
      void engine.fetchFile(attachment);
      return;
    }
    router.push({ pathname: "/chat-info/output/[id]", params: { id: output.id, chat: message.chat_id } });
  };
  if (stacked) {
    return (
      <Pressable
        onPress={open}
        style={[
          styles.stackRow,
          { width, backgroundColor: p.outputCard, borderColor: p.separator },
          !stack.above && styles.roundedAbove,
          !stack.below && styles.roundedBelow,
        ]}
        accessibilityRole="button"
        accessibilityLabel={spoken}
      >
        {stack.above && <View style={[styles.stackRule, { backgroundColor: p.separator }]} />}
        <View style={styles.stackIcon}>
          {kind === "image" && uri ? (
            <Image source={{ uri }} style={styles.thumbnail} contentFit="cover" transition={150} />
          ) : (
            <Symbol name={outputSymbol(output)} size={20} color={tint} />
          )}
        </View>
        <View style={{ flex: 1 }}>
          <Text style={[styles.cardName, { color: p.label }]} numberOfLines={1} ellipsizeMode="middle">
            {output.name}
          </Text>
          {detail ? (
            <Text style={[styles.cardDetail, { color: p.secondaryLabel }]} numberOfLines={1}>
              {detail}
            </Text>
          ) : null}
        </View>
        {evidence && <Symbol name={checkSymbol} size={15} color={checkTint} />}
      </Pressable>
    );
  }
  return (
    <Pressable
      onPress={open}
      style={[styles.card, { width, backgroundColor: p.outputCard, borderColor: p.separator }]}
      accessibilityRole="button"
      accessibilityLabel={spoken}
    >
      {preview !== undefined && (
        <View style={[styles.preview, { height: preview, backgroundColor: p.code, borderBottomColor: p.separator }]}>
          <Image source={uri ? { uri } : undefined} style={StyleSheet.absoluteFill} contentFit="cover" contentPosition="top" transition={150} />
        </View>
      )}
      <View style={styles.cardHeader}>
        <Symbol name={outputSymbol(output)} size={24} color={tint} />
        <View style={{ flex: 1 }}>
          <Text style={[styles.cardName, { color: p.label }]} numberOfLines={1} ellipsizeMode="middle">
            {output.name}
          </Text>
          {detail ? (
            <Text style={[styles.cardDetail, { color: p.secondaryLabel }]} numberOfLines={1}>
              {detail}
            </Text>
          ) : null}
        </View>
      </View>
      {evidence && (
        <View style={[styles.check, { borderTopColor: p.separator }]}>
          <Symbol name={checkSymbol} size={15} color={checkTint} style={styles.checkIcon} />
          <View style={{ flex: 1, gap: 2 }}>
            <Text style={[styles.checkTitle, { color: checkTint }]}>
              {evidenceTitle(evidence)} · {evidenceStatus(evidence)}
            </Text>
            {summary ? <Text style={[styles.checkSummary, { color: p.secondaryLabel }]}>{summary}</Text> : null}
            {evidence.command ? (
              <Text style={[styles.checkCommand, { color: p.tertiaryLabel }]} numberOfLines={1}>
                {evidence.command}
              </Text>
            ) : null}
          </View>
        </View>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  card: { borderRadius: 16, borderWidth: StyleSheet.hairlineWidth, overflow: "hidden" },
  preview: { width: "100%", borderBottomWidth: StyleSheet.hairlineWidth },
  stackRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    minHeight: 50,
    paddingHorizontal: 13,
    paddingVertical: 8,
    borderLeftWidth: StyleSheet.hairlineWidth,
    borderRightWidth: StyleSheet.hairlineWidth,
  },
  roundedAbove: { borderTopWidth: StyleSheet.hairlineWidth, borderTopLeftRadius: 16, borderTopRightRadius: 16 },
  roundedBelow: { borderBottomWidth: StyleSheet.hairlineWidth, borderBottomLeftRadius: 16, borderBottomRightRadius: 16 },
  // From the text's edge, as a list's rows are divided.
  stackRule: { position: "absolute", top: 0, left: 13 + STACK_ICON + 10, right: 0, height: StyleSheet.hairlineWidth },
  stackIcon: { width: STACK_ICON, height: STACK_ICON, alignItems: "center", justifyContent: "center" },
  thumbnail: { width: STACK_ICON, height: STACK_ICON, borderRadius: 5 },
  cardHeader: { flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: 13, paddingVertical: 11 },
  cardName: { fontSize: Font.message - 1, fontWeight: "600" },
  cardDetail: { fontSize: Font.small, marginTop: 1, fontVariant: ["tabular-nums"] },
  check: { flexDirection: "row", gap: 8, paddingHorizontal: 13, paddingTop: 10, paddingBottom: 11, borderTopWidth: StyleSheet.hairlineWidth },
  checkIcon: { marginTop: 1 },
  checkTitle: { fontSize: Font.small, fontWeight: "600" },
  checkSummary: { fontSize: Font.small, lineHeight: 18 },
  checkCommand: { fontSize: Font.caption, fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", marginTop: 1 },
});
