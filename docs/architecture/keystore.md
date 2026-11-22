# Keystore contract

How the master secret and serialized provider credentials could move from private files in the Beans home to a native OS keystore. See [Identity](identity.md) for the keys and credentials themselves.

## Contract

`keystore` exposes an inert byte-storage boundary for master secrets and serialized credentials. It is separate from the bots' encrypted [secret store](secrets.md) (`secret_store`). Production still reads and writes the private JSON files through `Config`, `App::save_identity` and `Credentials::save`; no native adapter is selected or invoked. The encrypted account blob, identity derivation and backup phrase flows keep their current formats.

`SecretId` identifies a secret kind within a caller-supplied namespace. Backend activation requires a stable namespace binding the admitted home and account, including separate development homes. The caller must hold exclusive account lifecycle admission. Native adapters must distinguish missing entries from locked or unavailable stores, create entries atomically without replacing conflicts, and report uncertain publication as `AmbiguousWrite`. Error variants carry no secret bytes or native error strings.

`copy_verified` rejects an empty namespace, refuses conflicting destination bytes, inserts only when absent, and reads the destination again for byte equality. A restart verifies an existing identical entry. A failed or ambiguous operation returns an error. The helper never touches a filesystem path and never deletes the source or its backup, even after a successful readback. This is a verified-copy primitive, not an activated file migration or a durability guarantee for an OS backend. Fake tests exercise both kinds, locked/unavailable reads, conflicting/corrupt writes and restart after ambiguous publication.

## Activation gates

Native activation remains gated on choosing and reviewing a pinned dependency's license and MSRV and qualifying macOS Keychain, Windows Credential Manager and Linux Secret Service separately. The integration must define namespace stability, secret payload formats, backend durability, lifecycle exclusion, backup recovery and account-forget deletion before switching callers. Source removal requires verified durable publication and a restart read from the selected backend while retaining a private recoverable backup. Locked desktop stores must fail visibly without silently selecting files. A Linux file backend is permitted only for an explicitly selected headless mode, with a visible risk notice and the existing private-file protections. No fallback is selected by this contract.
