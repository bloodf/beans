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

const checks = [
  { path: "/src/ui/motion.test.tsx", name: "checkReducedMotion", run: checkReducedMotion, transport: false },
  { path: "/src/ui/palette.test.tsx", name: "checkSearchResults", run: checkSearchResults, transport: false },
  { path: "/src/ui/overlay.test.tsx", name: "checkSheetDialogName", run: checkSheetDialogName, transport: false },
  { path: "/src/ui/sheets/skillDiscovery.test.tsx", name: "checkSkillDiscoveryConsumer", run: checkSkillDiscoveryConsumer, transport: true },
  { path: "/src/ui/sheets/skillLibrary.test.tsx", name: "checkSkillLibraryConsumer", run: checkSkillLibraryConsumer, transport: true },
  { path: "/src/ui/chat/cards.test.tsx", name: "checkCommandCardInput", run: checkCommandCardInput, transport: false },
  { path: "/src/ui/chat/composer.test.tsx", name: "checkComposerSendAck", run: checkComposerSendAck, transport: false },
  { path: "/src/ui/settings/deviceNaming.test.tsx", name: "checkDeviceNamingConsumer", run: checkDeviceNamingConsumer, transport: true },
];

declare global {
  interface Window { runFixture: typeof runFixture; }
}

export async function runFixture(fixture: { path: string; check: string; transport?: boolean }) {
  const selected = checks.find((check) => check.path === fixture.path && check.name === fixture.check && check.transport === (fixture.transport === true));
  if (!selected) throw new Error("Unknown browser fixture binding");
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
  if (selected.transport) hostInfo().isMock = false;
  const check = selected.run;
  const { promise, reject } = Promise.withResolvers<never>();
  const timer = setTimeout(() => reject(new Error("Browser check exceeded 30 seconds")), 30_000);
  try {
    return await Promise.race([check(), promise]);
  } finally {
    clearTimeout(timer);
  }
}
