// Vite compiles the fixture entries with the production Solid plugin.
import "./src/ui/theme.css";
import "./src/ui/components.css";
import "./src/ui/layout.css";
import { hostInfo, loadHost } from "./src/host";
import { setLanguage } from "./src/l10n";
import { store } from "./src/model/store";
import { checkReducedMotion } from "./src/ui/motion.test";
import { checkSearchResults } from "./src/ui/palette.test";
import { checkSheetDialogName } from "./src/ui/overlay.test";
import { checkSkillDiscoveryConsumer } from "./src/ui/sheets/skillDiscovery.test";
import { checkSkillLibraryConsumer } from "./src/ui/sheets/skillLibrary.test";
import { checkCommandCardInput } from "./src/ui/chat/cards.test";
import { checkComposerSendAck } from "./src/ui/chat/composer.test";
import { checkDeviceNamingConsumer } from "./src/ui/settings/deviceNaming.test";

const checks = { checkReducedMotion, checkSearchResults, checkSheetDialogName, checkSkillDiscoveryConsumer, checkSkillLibraryConsumer, checkCommandCardInput, checkComposerSendAck, checkDeviceNamingConsumer };
const paths: Record<keyof typeof checks, string> = {
  checkReducedMotion: "/src/ui/motion.test.tsx",
  checkSearchResults: "/src/ui/palette.test.tsx",
  checkSheetDialogName: "/src/ui/overlay.test.tsx",
  checkSkillDiscoveryConsumer: "/src/ui/sheets/skillDiscovery.test.tsx",
  checkSkillLibraryConsumer: "/src/ui/sheets/skillLibrary.test.tsx",
  checkCommandCardInput: "/src/ui/chat/cards.test.tsx",
  checkComposerSendAck: "/src/ui/chat/composer.test.tsx",
  checkDeviceNamingConsumer: "/src/ui/settings/deviceNaming.test.tsx",
};

declare global {
  interface Window { runFixture: typeof runFixture; }
}

export async function runFixture(fixture: { path: string; check: string; transport?: boolean }) {
  if (!Object.hasOwn(checks, fixture.check) || paths[fixture.check as keyof typeof checks] !== fixture.path) throw new Error(`Unknown browser fixture ${fixture.path} ${fixture.check}`);
  await loadHost();
  document.documentElement.dataset.platform = "linux";
  setLanguage("en");
  if (fixture.check === "checkSearchResults") {
    store.start();
    const deadline = Date.now() + 10_000;
    // Demo seed loads asynchronously; palette needs its seeded owning chat before invocation.
    while (store.chats.length === 0) {
      if (Date.now() >= deadline) throw new Error("Palette demo seed exceeded 10 seconds");
      const { promise, resolve } = Promise.withResolvers<void>();
      setTimeout(resolve, 0);
      await promise;
    }
  }
  if (fixture.transport) hostInfo().isMock = false;
  if (!(fixture.check in checks)) throw new Error(`Missing check ${fixture.check}`);
  const check = checks[fixture.check as keyof typeof checks];
  const { promise, reject } = Promise.withResolvers<never>();
  const timer = setTimeout(() => reject(new Error("Browser check exceeded 30 seconds")), 30_000);
  try {
    return await Promise.race([check(), promise]);
  } finally {
    clearTimeout(timer);
  }
}
