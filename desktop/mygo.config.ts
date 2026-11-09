import pkg from "../package.json" with { type: "json" };
import { RELEASES_REPO, TAG_PREFIX } from "../scripts/app.ts";
import { releasePublicKey } from "../scripts/release-signing.ts";

// Beans for Windows and Linux. `mygo dev` builds Beans Dev (app.beans.dev), which keeps its
// account in ~/.beans-dev-v2 and its CLI on port 4875, apart from an installed Beans.
export default async ({ command }: { command: string }) => ({
  name: "Beans",
  identifier: "app.beans",
  // All desktop artifacts share the root Beans release version.
  version: pkg.version,
  icon: command === "dev" ? "assets/icon-dev.png" : "assets/icon.png",
  devUrl: "http://localhost:5178",
  devCommand: "pnpm run dev:web",
  buildCommand: "pnpm run build:web",
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
  // Local opt-in MyGo packaging keeps its existing identifier and root version.
  macos: {
    signingIdentity: process.env.SIGN_IDENTITY,
    notarize: process.env.NOTARY_PROFILE
      ? { keychainProfile: process.env.NOTARY_PROFILE, keychain: process.env.NOTARY_KEYCHAIN }
      : undefined,
  },
  linux: {
    comment: "Chat with your bots, which run on computers you own",
    categories: ["Network", "Chat"],
  },
});
