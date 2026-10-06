import { defineConfig } from "mygo-cli";
import pkg from "../package.json" with { type: "json" };
import { RELEASES_REPO, TAG_PREFIX } from "../scripts/app.ts";
import { releasePublicKey } from "../scripts/release-signing.ts";

// Lorca for Windows and Linux. `mygo dev` builds Lorca Dev (app.lorca.dev), which keeps its
// account in ~/.lorca-dev and its CLI on port 4863, apart from an installed Lorca.
export default defineConfig(async ({ command }) => ({
  name: "Lorca",
  identifier: "app.lorca",
  // All desktop artifacts share the root Beans release version.
  version: pkg.version,
  icon: command === "dev" ? "assets/icon-dev.png" : "assets/icon.png",
  devUrl: "http://localhost:5178",
  devCommand: "bun run dev:web",
  buildCommand: "bun run build:web",
  frontendDist: "dist",
  bindings: "src/mygo.ts",
  out: "build",
  resources: ["../updates/public-key.txt"],
  // The existing Windows/Linux install identity stays unchanged; update trust is Beans-only.
  updates: command === "dev" ? undefined : {
    publicKey: await releasePublicKey(),
    github: RELEASES_REPO,
    tagPrefix: TAG_PREFIX,
    changelog: "../CHANGELOG.md",
    deltas: 0,
  },
  linux: {
    comment: "Chat with your bots, which run on computers you own",
    categories: ["Network", "Chat"],
  },
}));
