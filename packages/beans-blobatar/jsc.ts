// JavaScriptCore on macOS does not expose TextEncoder. Keep upstream's UTF-8 hash
// unchanged; supply its standard encoder before loading the vendored module.
if (typeof globalThis.TextEncoder === "undefined") {
  globalThis.TextEncoder = class {
    encode(text: string): Uint8Array {
      const bytes: number[] = [];
      for (let i = 0; i < text.length; i++) {
        let cp = text.charCodeAt(i);
        if (cp >= 0xd800 && cp <= 0xdbff) {
          const next = text.charCodeAt(i + 1);
          if (next >= 0xdc00 && next <= 0xdfff) {
            cp = 0x10000 + ((cp - 0xd800) << 10) + next - 0xdc00;
            i++;
          } else cp = 0xfffd;
        } else if (cp >= 0xdc00 && cp <= 0xdfff) cp = 0xfffd;
        if (cp < 0x80) bytes.push(cp);
        else if (cp < 0x800) bytes.push(0xc0 | (cp >> 6), 0x80 | (cp & 63));
        else if (cp < 0x10000) bytes.push(0xe0 | (cp >> 12), 0x80 | ((cp >> 6) & 63), 0x80 | (cp & 63));
        else bytes.push(0xf0 | (cp >> 18), 0x80 | ((cp >> 12) & 63), 0x80 | ((cp >> 6) & 63), 0x80 | (cp & 63));
      }
      return Uint8Array.from(bytes);
    }
  } as typeof TextEncoder;
}

import * as api from "./index";
// Every export is a pure function or a frozen list; install them as globals for Swift to call.
Object.assign(globalThis, api);
