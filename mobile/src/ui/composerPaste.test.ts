import { expect, test } from "bun:test";
import { namePastedImages, pasteTimestamp } from "./composerPaste";

test("namePastedImages maps MIME to extension and suffixes multi-image pastes", () => {
  const stamp = "2026-10-29 00.00.00";
  expect(
    namePastedImages([{ uri: "file://a", mime: "image/png", size: 10 }], stamp),
  ).toEqual([{ uri: "file://a", mime: "image/png", size: 10, name: `Pasted Image ${stamp}.png` }]);

  expect(
    namePastedImages(
      [
        { uri: "file://j", mime: "image/jpeg", size: 1, width: 2, height: 3 },
        { uri: "file://w", mime: "image/webp", size: 4 },
        { uri: "file://x", mime: "image/svg+xml", size: 5 },
        { uri: "file://z", mime: "image/", size: 6 },
      ],
      stamp,
    ),
  ).toEqual([
    { uri: "file://j", mime: "image/jpeg", size: 1, width: 2, height: 3, name: `Pasted Image ${stamp} 1.jpg` },
    { uri: "file://w", mime: "image/webp", size: 4, name: `Pasted Image ${stamp} 2.webp` },
    { uri: "file://x", mime: "image/svg+xml", size: 5, name: `Pasted Image ${stamp} 3.svg` },
    { uri: "file://z", mime: "image/", size: 6, name: `Pasted Image ${stamp} 4.png` },
  ]);
});

test("pasteTimestamp formats like Photo / Pasted Image stamps", () => {
  expect(pasteTimestamp(new Date("2026-10-29T12:34:56.789Z"))).toBe("2026-10-29 12.34.56");
});
