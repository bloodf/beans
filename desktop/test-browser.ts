import { chromium } from "playwright";
import type { Browser } from "playwright";
import { createServer } from "vite";

// Preserve fixture URLs and run each exported check in its own account/DOM/module context.
const fixtures = [
  { path: "/src/ui/motion.test.tsx", check: "checkReducedMotion", motion: "no-preference" },
  { path: "/src/ui/motion.test.tsx", check: "checkReducedMotion", motion: "reduce" },
  { path: "/src/ui/palette.test.tsx", check: "checkSearchResults" },
  { path: "/src/ui/overlay.test.tsx", check: "checkSheetDialogName" },
  { path: "/src/ui/sheets/skillDiscovery.test.tsx", check: "checkSkillDiscoveryConsumer", transport: true },
  { path: "/src/ui/sheets/skillLibrary.test.tsx", check: "checkSkillLibraryConsumer", transport: true },
  { path: "/src/ui/chat/cards.test.tsx", check: "checkCommandCardInput" },
  { path: "/src/ui/chat/composer.test.tsx", check: "checkComposerSendAck" },
  { path: "/src/ui/settings/deviceNaming.test.tsx", check: "checkDeviceNamingConsumer", transport: true },
] as const;

const server = await createServer({
  root: import.meta.dir,
  server: { host: "127.0.0.1", port: 0, strictPort: false, open: false },
  plugins: [{
    name: "browser-check-document",
    configureServer(vite) {
      vite.middlewares.use((request, response, next) => {
        if (request.url?.split("?")[0] !== "/__browser_checks__") return next();
        response.setHeader("Content-Type", "text/html");
        response.end('<!doctype html><html lang="en"><head><title>Beans browser checks</title></head><body><script type="module">import { runFixture } from "/browser-checks.ts"; window.runFixture = runFixture;</script></body></html>');
      });
    },
  }],
});
let browser: Browser | undefined;
let failures = 0;
try {
  await server.listen();
  const address = server.httpServer!.address();
  if (!address || typeof address === "string") throw new Error("Missing browser check server port");
  const origin = `http://127.0.0.1:${address.port}`;
  browser = await chromium.launch();
  for (const fixture of fixtures) {
    const context = await browser.newContext({
      viewport: { width: 1365, height: 900 }, locale: "en-US",
      reducedMotion: "motion" in fixture ? fixture.motion : "no-preference",
    });
    const label = `${fixture.path} ${"motion" in fixture ? fixture.motion : fixture.check}`;
    try {
      // These fixtures have synthetic transports only. No CLI, relay or external network is admitted.
      await context.route("**/*", (route) => new URL(route.request().url()).origin === origin ? route.continue() : route.abort());
      const page = await context.newPage();
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await page.goto(`${origin}/__browser_checks__?mock=1&platform=linux`);
      await page.waitForFunction(() => typeof window.runFixture === "function");
      if (fixture.check === "checkDeviceNamingConsumer") {
        const rejected = await page.evaluate(async (valid) => {
          const invalid = [
            { ...valid, path: "/src/ui/motion.test.tsx" },
            { ...valid, transport: false },
          ];
          for (const binding of invalid) {
            try { await window.runFixture(binding); }
            catch (error) {
              if (error instanceof Error && error.message === "Unknown browser fixture binding") continue;
              throw error;
            }
            throw new Error("Mismatched browser fixture was admitted");
          }
          return invalid.length;
        }, fixture);
        if (rejected !== 2) throw new Error("Browser fixture rejection checks did not run");
      }
      const result = await page.evaluate((fixture) => window.runFixture(fixture), fixture);
      if (errors.length) throw new Error(errors.join("\n"));
      console.log(`PASS ${label} ${JSON.stringify(result ?? "completed")}`);
    } catch (error) {
      failures++;
      console.error(`FAIL ${label}`, error);
    } finally {
      await context.close();
    }
  }
} finally {
  await browser?.close();
  await server.close();
}
if (failures) throw new Error(`${failures} mounted browser checks failed`);
console.log(`PASS all ${fixtures.length} mounted browser checks across 8 suites`);
