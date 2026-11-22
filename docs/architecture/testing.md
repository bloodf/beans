# Testing

How the code is tested, and the one command per stack that runs it.

## Commands

| Command | Runs |
| --- | --- |
| `pnpm run test:rust` | The Rust matrix below, stopping at the first failure |
| `pnpm run test:rust --quick` | The Runner's library tests and the ONNX Runtime tests |
| `pnpm run test:rust --live-hub` | Only the checks against the real model hub and PyPI (network) |
| `pnpm test` | Script, web and Blobatar tests (Vitest) |
| `pnpm --dir desktop test` | Desktop unit tests, then its browser checks (Playwright) |
| `pnpm --dir mobile test` | Phone tests against the mocked native core |
| `pnpm run test:e2e` | The end-to-end suites below (`--rust`, `--desktop`, `--web` pick one) |
| `pnpm --dir desktop run test:e2e`, `pnpm --dir web run test:e2e` | The desktop page's and the website's end-to-end checks in Chromium |
| `pnpm run check:docs`, `pnpm run l10n` | Doc budgets and links; every UI string translated |

`scripts/test-rust.ts` runs the Rust matrix in order:

1. The phone core compiles without the Runner (`--no-default-features --features provider-auth`), so nothing Runner-only leaks into it; a Runner compiles without local models (`--features runner`).
2. `cargo test -p beans` with default features (local models included), with the relay binary the revocation test drives (`BEANS_REVOCATION_RELAY_BIN`).
3. The tests that need a real ONNX Runtime, named `onnx_runtime_*` and ignored by default, with the pinned runtime (`scripts/onnxruntime.ts`) in `BEANS_ONNXRUNTIME`.
4. `cargo test -p beans-mobile`.
5. The catalog pin script's tests against a mock Hugging Face (`python3 -I -B scripts/schedule-intent-eval/test_pin_models.py`; tensor checks need `onnxruntime`).

A test that needs something the machine may lack is `#[ignore]`d with the reason, and this script supplies it. No test skips silently.

## End to end

Three suites drive whole systems through their real boundaries, with nothing mocked inside them:

- **Runner, Device and relay** (`crates/cli/tests/e2e_paired_chat.rs`): a real `beans-relay` and two `beans serve` processes with their own homes, driven over the local websocket API the apps use, with a local OpenAI-compatible model server standing in for the provider. A Device pairs with a new account's Runner; the Runner connects the model server as a custom provider and creates a bot; messages from either side reach the bot and its reply reaches the Device through the relay; the relay's database and files hold no plaintext of the messages, bot profile, provider name or key. It also restarts the Runner on its home (history kept, answering again), unpairs the Device (it leaves every roster and forgets the account), and sends a secret the Runner's environment holds from the Device: the model server receives only `{{secret:OPENAI_API_KEY}}`, never the value in any form (the automated part of #553). The tests are `#[ignore]`d because they need the relay binary in `BEANS_E2E_RELAY_BIN`; `pnpm run test:rust` and `pnpm run test:e2e` build and supply it.
- **Desktop page** (`desktop/e2e`): the production page in Chromium against its own Vite dev server, over the demo workspace (`?mock=1`): sending and the demo reply, the composer, the search palette and its actions, the demo's Related and Pictures results and Settings › Search, Settings panes, switching the app language, New Bot, a chat's context menu, the sidebar toggle. A page error or console error fails a check.
- **Website** (`web/e2e`): the site in Chromium against its Vite dev server, after `build:preview` builds the desktop demo it embeds: the home page and its live preview, docs navigation, `/zh`, the download page, the install scripts, `llms.txt`, the served catalogs and not-found.

CI (`.github/workflows/test.yml`, run by manual dispatch) runs all of these: the Rust jobs run the Runner/Device/relay suite, the desktop and website jobs run theirs, and a repository job runs `pnpm test`, `pnpm run check:docs` and `pnpm run l10n`.

The phone and the native macOS and Windows/Linux windows need a device, a simulator or the platform's toolchain; their end-to-end acceptance is manual.

## Conventions

- **SQL lives in files.** Every statement of the local store is `crates/cli/src/local_store/sql/<area>/<name>.sql`, embedded with `sql!("area/name")`: a missing file fails the build. `local_store::tests::sql` prepares every file against a fresh schema, fails on files no code uses, and upgrades an older store's columns. Tests keep inline SQL only where it stands for another binary's writes, which the store must survive.
- **Oracles for rewrites.** A change that must keep behaviour compares itself with the old algorithm kept in the test (`tests/local_search.rs` runs the decode-everything search beside the indexed one over randomized Unicode data). Mutating the new code must fail the oracle.
- **Fixtures behave like the real thing where it matters.** Semantic search uses a synonym-concept text model, a colour-reading image tower, and a scripted vision provider (`search_index::tests`). The static model test uses a real safetensors table and tokenizer. ONNX tests load tiny models with real tensor contracts, generated by `crates/cli/src/embeddings/fixtures/make_onnx.py` (needs the `onnx` Python package) and run on the real runtime.
- **No test reaches the network.** Model downloads run against `schedule_models/mock_hub.rs`, a local hub with a CDN redirect that can answer with any failure; the Runner's computer is set per test (`set_machine`), so suggestions do not depend on the machine running them. Pointing a real Runner at a mirror or a local hub is `BEANS_MODEL_HUB`. The only tests that reach the network are `live_hub_*`, ignored and run only by `--live-hub`.
- **Edges are tested where they break things.** Each rule gets its limits: empty, one, exactly the limit, one past it, unknown, non-finite, overflow, and data an earlier release saved. Time-dependent code (retry backoff) runs on a paused clock (`#[tokio::test(start_paused = true)]`).
- **Plans are asserted.** Queries that must stay cheap at scale (queues, exact-search candidates) assert their `EXPLAIN QUERY PLAN` uses their index and no temporary sort.
- **Shared state has one implementation.** Per-account settings files and background model loads (`runner_state::{AccountFile, Loader}`) are tested once, for every user of them.

## Adding to the store

1. Write the statement in `sql/<area>/<name>.sql`, with a comment for any parameter that is not obvious. Name it for what it does (`messages/page_before`); a function with several statements names each `<function>_<verb>_<table>`. A statement two functions share is one file.
2. Call it with `sql!("<area>/<name>")` from the area's Rust file (`tasks`, `messages`, `state`, `outbox`, `search`, `vectors`).
3. A new column on a shipped table is a `schema/add_<table>_<column>.sql` entry in `ADDED_COLUMNS`. A change to a derived search table instead bumps `SEARCH_INDEX_VERSION`, which rebuilds them.
4. `cargo test -p beans --lib local_store` proves it prepares, is used, and upgrades.
