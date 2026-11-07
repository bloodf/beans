# TinyCortex evaluation

Date: 2026-11-07. Issue: [#99](https://github.com/bloodf/beans/issues/99).
Outcome: **do-not-import**.

This decision rejects direct import of the pinned engine and its retrieval/provenance components into Beans. It implements no runtime change. A future adapter proposal needs its own security proof and baseline; this decision does not authorize it.

## Source and license

The evaluated project is tinyhumansai/tinycortex at `72ce1d176aa3dc3a9ece14b9e61ecef9a8ae6b53`, not CortexDB Memory v2. Primary sources at that exact revision:

- [Manifest](https://github.com/tinyhumansai/tinycortex/blob/72ce1d176aa3dc3a9ece14b9e61ecef9a8ae6b53/Cargo.toml).
- [Complete license text](https://github.com/tinyhumansai/tinycortex/blob/72ce1d176aa3dc3a9ece14b9e61ecef9a8ae6b53/LICENSE), read before this recommendation. The manifest declares `GPL-3.0-only`; LICENSE contains GNU GPL version 3, 29 June 2007. Beans also declares GPL-3.0-only. Licensing is not the rejection reason. No upstream code is copied by this decision.
- [Memory module](https://github.com/tinyhumansai/tinycortex/blob/72ce1d176aa3dc3a9ece14b9e61ecef9a8ae6b53/src/memory/mod.rs), [global retrieval](https://github.com/tinyhumansai/tinycortex/blob/72ce1d176aa3dc3a9ece14b9e61ecef9a8ae6b53/src/memory/retrieval/global.rs), and [source retrieval](https://github.com/tinyhumansai/tinycortex/blob/72ce1d176aa3dc3a9ece14b9e61ecef9a8ae6b53/src/memory/retrieval/source.rs).

## Safety stop before prototype

`query_global` calls `collect_source_hits(config, None, source_kind, window)`. Source retrieval selects all source trees when neither source id nor kind narrows the selection. Its arguments contain no Beans account/bot authorization context. A source kind or provenance taint is not an own-bot authorization check. Importing this path over a shared account workspace cannot establish Beans' own-bot boundary. This is a source-level integration incompatibility, not a demonstrated exploit of TinyCortex or Beans. Separate host-owned workspaces could address scope, but that requires an adapter and lifecycle proof that do not exist in this proposal.

The source collector also documents that unwindowed retrieval materializes all selected summaries before the result limit. A returned-hit cap alone does not prove Beans' recall resource budget. Direct substitution is rejected before allocating a prototype or running private content through the engine.

TinyCortex documents local Markdown and SQLite under one configured workspace, atomic writes, and fail-closed decoding of unknown persisted taint. These are useful mechanisms. They do not prove preservation of Beans' canonical `MEMORY.md`, topic notes and dated logs, nor rebuilding a deleted index without losing notes. Its [content paths](https://github.com/tinyhumansai/tinycortex/blob/72ce1d176aa3dc3a9ece14b9e61ecef9a8ae6b53/src/memory/store/content/paths.rs) use per-chunk Markdown and derived wiki summaries. No Beans migration or index-deletion exercise ran.

The [safety wrapper](https://github.com/tinyhumansai/tinycortex/blob/72ce1d176aa3dc3a9ece14b9e61ecef9a8ae6b53/src/memory/store/safety/mod.rs) delegates scrubbing to tinymemory-safety. It is not evidence of equivalence to Beans' credential scrub. Existing scrub, account encryption, own-bot namespaces, consent, durable queues and deletion fences remain unchanged. No real user data, secrets, retain/reflect request, paid service or OS keystore was used.

## Features and dependency cost

`default = []`. The synchronous core does not require an account or mandatory remote service. The manifest has no OpenHuman runtime dependency; ported source and comments about host version alignment do not mean that runtime is pulled in. Required-service rejection therefore does not apply.

Optional features are `tokio`, `git-diff`, `providers-http`, `sync`, `persona`, `wiki-git`, `obsidian`, `people`, and `contacts`. HTTP providers and sync enable reqwest and Tokio; git features enable vendored libgit2; contacts adds macOS address-book bindings. None is authorized here.

Even with default features disabled, direct normal dependencies include anyhow, log, futures, async-trait, chrono, parking_lot, rand, regex, rusqlite with bundled SQLite, serde, serde_json, schemars, sha2, thiserror, toml, uuid, walkdir, and these mandatory external contracts:

| Edge | Pinned source |
| --- | --- |
| tinymemory-safety | tinymemory `b49650e1eac6d55a41f641f4ec8a683a25df9518` |
| tinyinference-embeddings and tinyinference-llm | tinyinference `92445ea582c9b1c5dc78891ebbff4a817f9e8fa9` |
| tinycortex-api | local `api/`, which requires tinymemory-api at tinymemory `1a03a00c11f68beea63a743bb369d947f98fcde1` |

The [API manifest](https://github.com/tinyhumansai/tinycortex/blob/72ce1d176aa3dc3a9ece14b9e61ecef9a8ae6b53/api/Cargo.toml) confirms the last edge. The [safety manifest](https://github.com/tinyhumansai/tinymemory/blob/b49650e1eac6d55a41f641f4ec8a683a25df9518/crates/tinymemory-safety/Cargo.toml) requires Rust 1.96 and regex, serde_json and log. This is a static dependency inventory, not a resolved transitive closure, compatibility pass, package count or binary measurement. Remaining transitive resolution and licensing are NOT RUN and are not approved by this decision. Existing package overlap cannot establish Cargo version unification.

Beans already has local lexical/time recall, the MemoryBackend capability boundary, embeddings with pinned vector-space metadata, and Lance/pgvector plus remote adapters. TinyCortex adds another content/tree/index pipeline and embedding/LLM contracts. Its provenance distinction does not replace authorization, and no measured incremental benefit justifies bridging those contracts or replacing existing storage.

## Actual baseline and measurement boundary

At Beans main `1a1386a15696a50eddff915036f75cde2675203b`, `crates/cli/tests/memory_retrieval_fixture.rs` and `fixtures/memory_retrieval.json` come from `490771fcabf2713aafe940e50b953dcbd0f7d6fc`, merged by `ad2c88cb93b07d3ffcf8b3f7276c1937da6e705d` in [PR #258](https://github.com/bloodf/beans/pull/258). The test explicitly says it does not invoke retrieval backends. Its fixture declares artificial vectors and scripted remote replies. PR #258 records three passing corpus/scorer tests and explicitly leaves the five-path runtime harness and baseline acceptance open. That is historical fixture evidence, not a fresh run or a backend-quality result. Issue #98 being closed does not change that boundary.

Foreign-canary scope checks and expected rejection in that fixture are preserved byte-for-byte. No fixture input was repurposed as measured retrieval output. The coordinator was asked for an exact runtime artifact if one exists; the safety rejection does not depend on receiving it.

| Quantity | Result |
| --- | --- |
| Beans versus TinyCortex hit@1/hit@5 | NOT RUN |
| Latency, throughput and memory | NOT RUN |
| Release binary size and size delta | NOT RUN |
| Resolved transitive dependency delta | NOT RUN |
| Index deletion/rebuild and foreign-bot runtime isolation | NOT RUN |

No zero values, speedup, quality gain or savings are inferred. No upstream benchmark is a Beans result. The authorized early safety stop replaces prototype measurement for this rejection; it does not satisfy the original measurement boxes by implication.

## Verification and effect

Manual source review on macOS, 2026-11-07, supports the direct-import rejection above. Only this decision record changes product documentation; current mechanisms in architecture docs remain accurate. Cargo manifests, features, lockfiles, credentials, baseline fixtures and production code are unchanged. Rust unit tests, lint and builds are N/A for this documentation-only decision and were not run. Documentation validation is recorded with the PR evidence. Adoption, partial adoption and dependent production work remain unauthorized by this record.
