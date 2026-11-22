// One thing a bot left for review, slid in from the chat's details. Attention rows that wait on
// an open review open this screen over the chat. Full approve/reject against the Runner lands
// with the review-queue port; this screen shows what the phone already has.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { useReview } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { Section } from "../../../src/ui/forms";
import { CloseToolbar } from "../../../src/ui/navigation";
import { Font, usePalette } from "../../../src/ui/theme";

export default function ReviewScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const review = useReview(id);
  if (!review) {
    return (
      <>
        <Stack.Screen options={{ title: t("Review") }} />
        <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.back()} />
        <Text style={[styles.empty, { color: p.secondaryLabel }]}>{t("This review is no longer available")}</Text>
      </>
    );
  }
  const body =
    review.payload.kind === "draft"
      ? review.payload.text
      : review.payload.kind === "shell"
        ? review.payload.arguments.command ?? ""
        : `${review.payload.server_name}.${review.payload.tool}`;
  return (
    <>
      <Stack.Screen options={{ title: t("Review") }} />
      <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.back()} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content}>
        <Section>
          <View style={styles.block}>
            <Text style={[styles.label, { color: p.secondaryLabel }]}>{t("Next")}</Text>
            <Text style={[styles.body, { color: p.label }]}>{review.rationale}</Text>
          </View>
          {body ? (
            <View style={styles.block}>
              <Text style={[styles.label, { color: p.secondaryLabel }]}>{review.payload.kind === "draft" ? t("Messages") : t("Review")}</Text>
              <Text style={[styles.body, { color: p.label, fontFamily: review.payload.kind === "draft" ? undefined : "monospace" }]}>{body}</Text>
            </View>
          ) : null}
          <View style={styles.block}>
            <Text style={[styles.label, { color: p.secondaryLabel }]}>{review.state}</Text>
            <Text style={[styles.body, { color: p.secondaryLabel }]}>{review.target.resource}</Text>
          </View>
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  empty: { textAlign: "center", marginTop: 32, fontSize: 15 },
  block: { paddingHorizontal: 16, paddingVertical: 12, gap: 4 },
  label: { fontSize: Font.small },
  body: { fontSize: Font.body, lineHeight: 22 },
});
