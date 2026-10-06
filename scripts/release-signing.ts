import { createPrivateKey, createPublicKey, sign } from "node:crypto";
import type { KeyObject } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** The installed trust anchor is public; publishing keys never come from an upstream keychain. */
export async function releasePublicKey(): Promise<string> {
  const value = (await readFile(join(root, "updates/public-key.txt"), "utf8")).trim();
  if (!/^[A-Za-z0-9+/]{43}=$/.test(value) || Buffer.from(value, "base64").length !== 32) {
    throw new Error("updates/public-key.txt must contain a raw Ed25519 public key");
  }
  return value;
}

export async function releasePrivateKey(): Promise<KeyObject> {
  const pem = process.env.BEANS_UPDATE_PRIVATE_KEY;
  if (!pem) throw new Error("BEANS_UPDATE_PRIVATE_KEY must contain the Beans Ed25519 PKCS8 PEM signing key");
  const key = createPrivateKey(pem);
  if (key.asymmetricKeyType !== "ed25519") throw new Error("Release signing requires an Ed25519 key");
  const der = createPublicKey(key).export({ format: "der", type: "spki" });
  if (der.subarray(-32).toString("base64") !== await releasePublicKey()) {
    throw new Error("Release signing key does not match the installed Beans public key");
  }
  return key;
}

/** MyGo and Sparkle use libsodium's seed32 + public32 representation. */
export async function rawReleaseSecret(): Promise<string> {
  const key = await releasePrivateKey();
  const privateDer = key.export({ format: "der", type: "pkcs8" });
  const publicDer = createPublicKey(key).export({ format: "der", type: "spki" });
  return Buffer.concat([privateDer.subarray(-32), publicDer.subarray(-32)]).toString("base64");
}

export async function signReleaseBytes(bytes: Uint8Array): Promise<string> {
  return sign(null, bytes, await releasePrivateKey()).toString("base64");
}

/** Never pass a private key in command-line arguments or leave it in build output. */
export async function withReleaseKeyFile<T>(run: (path: string) => Promise<T>): Promise<T> {
  const directory = await mkdtemp(join(tmpdir(), "beans-release-key-"));
  const path = join(directory, "key");
  try {
    await writeFile(path, `${await rawReleaseSecret()}\n`, { mode: 0o600, flag: "wx" });
    return await run(path);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}
