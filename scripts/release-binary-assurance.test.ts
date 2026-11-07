import * as trust from "./release-build-inputs.ts";
import { existsSync } from "node:fs";
import { expect, vi, test } from "vitest";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readVersion } from "./app.ts";
import { stageEasBuild, verifyEasBinary } from "./release-eas.ts";
import { inspectMobileBinary, type PublicTrustPolicy } from "./release-build-inputs.ts";

test("missing public authority rejects every profile before reading or downloading candidate bytes", async () => {
  const root = await mkdtemp(join(tmpdir(), "beans-binary-assurance-"));
  const revision = "a".repeat(40), project = "11111111-1111-4111-8111-111111111111";
  const previous = process.env.BEANS_EXPO_PROJECT_ID;
  process.env.BEANS_EXPO_PROJECT_ID = project;
  let downloaded = false;
  const fetcher = vi.spyOn(globalThis, "fetch").mockImplementation(async () => {
    downloaded = true;
    throw new Error("Candidate transport must not run without public authority");
  });
  try {
    for (const profile of ["github", "production", "testflight"] as const) {
      const build = { id: "22222222-2222-4222-8222-222222222222", status: "FINISHED", platform: profile === "testflight" ? "IOS" : "ANDROID", appIdentifier: "ai.amoena.beans", distribution: "STORE", buildProfile: profile, gitCommitHash: revision, appVersion: readVersion(), appBuildVersion: "42", app: { id: project }, artifacts: { buildUrl: "https://artifacts.eascdn.net/fixture.apk" } };
      await expect(stageEasBuild(build, profile, revision, join(root, profile))).rejects.toThrow(`policy unavailable for ${profile}`);
      expect(() => inspectMobileBinary(join(root, "absent-candidate"), profile, readVersion(), "42")).toThrow(`policy unavailable for ${profile}`);
      expect(existsSync(join(root, profile, `eas-${profile}.json`))).toBe(false);
    }
    expect(downloaded).toBe(false);
  } finally {
    fetcher.mockRestore();
    if (previous === undefined) delete process.env.BEANS_EXPO_PROJECT_ID; else process.env.BEANS_EXPO_PROJECT_ID = previous;
    await rm(root, { recursive: true, force: true });
  }
});


test("admitted APK and AAB candidates reach archive inspection after temporary staging", async () => {
  // Isolate temporary-file staging from native tool availability. No signature acceptance is mocked.
  const admission = vi.spyOn(trust, "preflightMobileTrust").mockImplementation((profile, policy) => trust.validatePublicTrustPolicy(policy, profile));
  try {
    const policies: PublicTrustPolicy[] = [
      { profile: "github", signerSha256: ["a".repeat(64)], rotation: "none" },
      { profile: "production", uploadSignerSha256: "b".repeat(64), rotation: "none" },
    ];
    for (const policy of policies) {
      await expect(verifyEasBinary(new Uint8Array([0x50, 0x4b]), policy.profile, readVersion(), "42", policy)).rejects.toThrow("Unsafe binary archive structure or decompressed size");
    }
  } finally { admission.mockRestore(); }
});
