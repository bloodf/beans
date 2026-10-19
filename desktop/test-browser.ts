import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium } from "playwright";
import type { Browser } from "playwright";
import { createServer } from "vite";
import type { ViteDevServer } from "vite";

const expected: Record<string, string> = {
  "src/ui/chat/cards.browser.tsx": "checkCommandCardInput",
  "src/ui/chat/composer.browser.tsx": "checkComposerSendAck",
  "src/ui/motion.browser.tsx": "checkReducedMotion",
  "src/ui/overlay.browser.tsx": "checkSheetDialogName",
  "src/ui/palette.browser.tsx": "checkSearchResults",
  "src/ui/sheets/skillDiscovery.browser.tsx": "checkSkillDiscoveryConsumer",
  "src/ui/sheets/skillLibrary.browser.tsx": "checkSkillLibraryConsumer",
};
const files = [...new Bun.Glob("src/**/*.browser.tsx").scanSync()].sort();
if (Object.keys(expected).some((file) => !files.includes(file))) throw new Error("Required browser consumer missing");
const evidence = process.env.BROWSER_EVIDENCE_DIR;
if (evidence) await mkdir(evidence, { recursive: true });
const cache = await mkdtemp(join(tmpdir(), "beans-browser-"));
let server: ViteDevServer | undefined;
let browser: Browser | undefined;
let failures = 0;
try {
  server = await createServer({
    cacheDir: cache,
    server: { host: "127.0.0.1", port: 0, strictPort: false, open: false },
    plugins: [{
      name: "browser-consumer-fixture",
      configureServer(server) {
        server.middlewares.use((request, response, next) => {
          if (request.url?.split("?")[0] !== "/__browser_check") return next();
          response.setHeader("Content-Type", "text/html");
          response.end('<!doctype html><html data-platform="linux"><head><link rel="icon" href="data:,"></head><body></body></html>');
        });
      },
    }],
  });
  await server.listen();
  const address = server.httpServer?.address();
  if (!address || typeof address === "string") throw new Error("Browser fixture has no listening port");
  browser = await chromium.launch({ headless: true });
  for (const file of files) {
    const settings = file.endsWith("/motion.browser.tsx") ? ["reduce", "no-preference"] as const : ["no-preference"] as const;
    for (const reducedMotion of settings) {
      const context = await browser.newContext({ reducedMotion, viewport: { width: 1200, height: 900 } });
      let timer: ReturnType<typeof setTimeout> | undefined;
      try {
        const run = async () => {
          const page = await context.newPage();
          page.setDefaultTimeout(30_000);
          const errors: string[] = [];
          page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
          page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
          const mock = file.includes("/sheets/skill") ? "0" : "1";
          await page.goto(`http://127.0.0.1:${address.port}/__browser_check?mock=${mock}&platform=linux`);
          await page.evaluate(async () => {
            for (const href of ["/src/ui/theme.css", "/src/ui/components.css", "/src/ui/layout.css"]) {
              await new Promise<void>((resolve, reject) => {
                const link = document.createElement("link");
                link.rel = "stylesheet"; link.href = href;
                link.onload = () => resolve(); link.onerror = () => reject(new Error(`Stylesheet failed: ${href}`));
                document.head.append(link);
              });
            }
          });
          if (file.endsWith("/palette.browser.tsx")) {
            // Browser-only state loads in the isolated page, never in the Bun runner.
            await page.evaluate(async (path) => { const { store } = await import(path); store.start(); }, "/src/model/store.ts");
          }
          const result = page.evaluate(async ({ modulePath, required }) => {
            const module = await import(`/${modulePath}`);
            const checks = Object.entries(module).filter(([name]) => name.startsWith("check"));
            if (!checks.length || checks.some(([, check]) => typeof check !== "function")) throw new Error(`Missing callable checks: ${modulePath}`);
            if (required && typeof module[required] !== "function") throw new Error(`Missing required export: ${required}`);
            const results: Record<string, unknown> = {};
            for (const [name, check] of checks) results[name] = (await (check as () => unknown)()) ?? null;
            return results;
          }, { modulePath: file, required: expected[file] });
          // Observe the real composer while its asynchronous acknowledgement scenarios run.
          if (evidence && file.endsWith("/composer.browser.tsx")) {
            const mounted = page.locator("textarea").first().waitFor({ state: "visible" });
            await Promise.race([mounted, result]);
            if (await page.locator("textarea").count()) await page.screenshot({ path: join(evidence, "composer-mounted.png") });
          }
          const output = await result;
          if (errors.length) throw new Error(errors.join("\n"));
          console.log(JSON.stringify({ file, reducedMotion, status: "pass", result: output }));
        };
        // Include page setup and module/style loading in the consumer deadline.
        await Promise.race([run(), new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error(`Consumer deadline: ${file}`)), 30_000);
          timer.unref();
        })]);
      } catch (error) {
        failures++;
        console.error(JSON.stringify({ file, reducedMotion, status: "fail", error: error instanceof Error ? error.stack : String(error) }));
      } finally {
        if (timer) clearTimeout(timer);
        await context.close();
      }
    }
  }
  if (failures) throw new Error(`${failures} browser consumer invocation(s) failed`);
} finally {
  try { await browser?.close(); } finally {
    try { await server?.close(); } finally { await rm(cache, { recursive: true, force: true }); }
  }
}
