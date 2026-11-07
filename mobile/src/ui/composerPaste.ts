// Pure naming for images pasted into the composer: MIME → extension and a multi-image suffix.
// Kept free of native modules so unit tests can cover it without ImagePasteView.

export interface PastableImage {
  uri: string;
  mime: string;
  size: number;
  width?: number;
  height?: number;
}

export function pasteTimestamp(now = new Date()): string {
  return now.toISOString().slice(0, 19).replace("T", " ").replace(/:/g, ".");
}

/// Turns pasted image payloads into attachment files named like picked photos.
export function namePastedImages<T extends PastableImage>(images: T[], stamp: string): Array<T & { name: string }> {
  return images.map((image, index) => {
    const extension = image.mime === "image/jpeg" ? "jpg" : image.mime.slice("image/".length).split("+")[0] || "png";
    const suffix = images.length > 1 ? ` ${index + 1}` : "";
    return { ...image, name: `Pasted Image ${stamp}${suffix}.${extension}` };
  });
}
