// A message body as a native text view. The core parses the Markdown; the view renders it
// selectable in place (UITextView on iOS, TextView on Android), sizes itself to its text within
// `maxWidth`, and opens links.
import { requireNativeModule, requireNativeViewManager } from "expo-modules-core";
import { PixelRatio, type ProcessedColorValue, type StyleProp, type ViewStyle } from "react-native";

export interface MarkdownViewProps {
  markdown: string;
  /// The widest the text may run, in points; the view claims the width it actually uses.
  maxWidth: number;
  fontSize: number;
  codeFontSize: number;
  color: ProcessedColorValue;
  linkColor: ProcessedColorValue;
  codeBackground: ProcessedColorValue;
  quoteColor: ProcessedColorValue;
  /// Table rules.
  border: ProcessedColorValue;
  /// Selection handles and highlight.
  tint: ProcessedColorValue;
  style?: StyleProp<ViewStyle>;
}

const native = requireNativeModule<{ measure?(markdown: string, maxWidth: number, fontSize: number, codeFontSize: number): { width: number; height: number } }>("MarkdownView");

// Bound both entry count and retained text during streaming. Longer messages still measure.
const measurements = new Map<string, { width: number; height: number }>();
const measurementLimit = 128;
const textLimit = 16_384;

/// The size the text takes within `maxWidth`, measured natively before the view renders. iOS
/// sizes the view from this; on Android the view claims its own size.
export function measureMarkdown(markdown: string, maxWidth: number, fontSize: number, codeFontSize: number) {
  if (!native.measure) return undefined;
  if (markdown.length > textLimit) return native.measure(markdown, maxWidth, fontSize, codeFontSize);
  const key = JSON.stringify([markdown, maxWidth, fontSize, codeFontSize, PixelRatio.getFontScale()]);
  const cached = measurements.get(key);
  if (cached) {
    measurements.delete(key);
    measurements.set(key, cached);
    return cached;
  }
  // Failed or unavailable measurements never enter the cache; native errors still propagate.
  const measured = native.measure(markdown, maxWidth, fontSize, codeFontSize);
  if (measured) {
    if (measurements.size === measurementLimit) measurements.delete(measurements.keys().next().value!);
    measurements.set(key, measured);
  }
  return measured;
}

export const MarkdownView = requireNativeViewManager<MarkdownViewProps>("MarkdownView");
