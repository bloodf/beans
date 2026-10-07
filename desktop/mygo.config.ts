import { defineConfig } from "mygo-cli";
import pkg from "../package.json" with { type: "json" };
import { RELEASES_REPO, TAG_PREFIX } from "../scripts/app.ts";
import { releasePublicKey } from "../scripts/release-signing.ts";

// Beans for Windows and Linux. `mygo dev` builds Beans Dev (app.beans.dev), which keeps its
// account in ~/.beans-dev-v2 and its CLI on port 4875, apart from an installed Beans.
export default defineConfig(async ({ command }) => ({
  name: "Beans",
  identifier: "app.beans",
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
