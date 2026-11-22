// One Vite dev server and one Chromium for an e2e file. Pages open the demo workspace, which runs
// the real store and views over the built-in demo engine instead of a CLI.
import { chromium, type Browser, type Page } from "playwright";
import { createServer, type ViteDevServer } from "vite";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));

export interface Harness {
  server: ViteDevServer;
  browser: Browser;
  base: string;
  open(query?: string): Promise<{ page: Page; errors: string[] }>;
  close(): Promise<void>;
}

export async function start(): Promise<Harness> {
  const server = await createServer({ root, configFile: `${root}/vite.config.ts`, logLevel: "error", server: { port: 0, strictPort: false, host: "127.0.0.1" } });
  await server.listen();
  const address = server.httpServer?.address();
  if (!address || typeof address === "string") throw new Error("Vite did not bind a TCP port");
  const base = `http://127.0.0.1:${address.port}`;
  const browser = await chromium.launch();
  return {
    server,
    browser,
    base,
    async open(query = "?mock=1") {
      const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, locale: "en-US" });
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(String(error)));
      page.on("console", (message) => { if (message.type() === "error") errors.push(`console: ${message.text()}`); });
      await page.goto(`${base}/${query}`, { waitUntil: "networkidle" });
      await page.getByRole("listbox").first().waitFor();
      return { page, errors };
    },
    async close() {
      await browser.close();
      await server.close();
    },
  };
}
