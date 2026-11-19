# Schedule models

A Runner can notice that a user's message asks for something to happen regularly ("summarize my inbox and run this daily", "agende para esse prompt rodar diariamente") before the bot's turn, and tell the bot in a note (`crates/cli/src/schedule_intent.rs`). The bot still sets the routine up with its routines tool; a note only points at the request.

The read can use the small model of the bot's provider, which costs tokens, or a local embedding model the user picked from a fixed catalog, which runs on the Runner's CPU and costs none. This doc covers the local models: `crates/cli/src/schedule_models/`.

## Detection modes

Each Runner keeps one mode, set with `schedule_models.select`:

| Mode | What reads the message | Tokens |
| --- | --- | --- |
| `reader` (default) | The small model of the bot's provider: quotes the ask and writes the schedule | Yes |
| `local` | The selected local model alone; a note quotes the clause it flagged and leaves the schedule to the bot | No |
| `local_then_reader` | The local model first; the small model only for a message it flags | Only for flagged messages |
| `off` | Nothing; the bot still reads the message itself | No |

Only a user's own message that starts a turn in a direct chat is read. A local mode whose model is not loaded, missing, or failed reads nothing rather than fall back to tokens; a selected model not loaded yet starts loading in the background for the next message. A local read waits at most 3 seconds.

`local` flags a message whose classifier score (log-odds) is above 0. `local_then_reader` passes one above −1 on to the small model, since a message it stops gets no note at all.

## Catalog

`catalog.json` is built in. Each model is at most 1 GB with its tokenizer (`MAX_MODEL_BYTES`), runs on one CPU thread, and names its Hugging Face repository, revision, files, license, languages, and engine (`kind` in listings: `text` or `image`). `rank` (1–9) orders models of equal fit for [suggestions](#suggestions), and `multilingual` marks a model that reads more than English:

- `static`: a token embedding table (safetensors) and its tokenizer. A text's vector is the unit-length mean of its tokens' rows. No native runtime.
- `onnx`: a transformer through ONNX Runtime, with the same pinned contract as local memory embeddings (tensor names, pooling, padding, maximum tokens), and optional `query_prefix`/`document_prefix` for models trained with them.
- `clip`: an image model for [semantic search](semantic-search.md): a pooled text tower (`model`, its `onnx` spec) and a vision tower (`image_model`, its `image` spec: input and output names, square input size, per-channel mean and std). It reads no schedules.

ONNX and image models run on the ONNX Runtime shipped beside the CLI (1.31.1, CPU, from the pinned official wheels in `crates/cli/onnxruntime.json`), or a library the user names. A Runner with none beside it (a CLI installed on its own, a development build) downloads the same pinned wheel with the model, as part of the preview the user approves (`embeddings/runtime_download.rs`): the wheel is checked against its size and SHA-256, the library is unpacked in memory (`embeddings/archive.rs`: stored or deflated members only, never more than the pinned size) and checked against its own, and only the library is kept. Each install keeps its own copy; one process loads the runtime once and accepts any copy with the same bytes, so a text and an image model run side by side.

| Id | Engine | Size | Languages |
| --- | --- | --- | --- |
| `wordllama-l2-256` | static | 18 MB | English and other Latin-script languages; weak in Chinese and Japanese |
| `wordllama-l3-128` | static | ~42 MB | Llama 3 tokenizer; trained on English |
| `wordllama-l3-256` | static | ~75 MB | Llama 3 tokenizer; trained on English |
| `potion-base-8m` | static | ~32 MB | English |
| `all-minilm-l6-v2-int8` | onnx | ~24 MB | English |
| `bge-small-en-v1.5-int8` | onnx | ~35 MB | English |
| `multilingual-e5-small-int8` | onnx | ~135 MB | About 100 languages, Chinese and Japanese included |
| `paraphrase-multilingual-minilm-l12-int8` | onnx | ~135 MB | About 50 languages, Chinese and Japanese included |
| `clip-vit-b32-int8` | clip | ~160 MB | English queries; pictures by what they show |

The multilingual and image entries are not pinned yet: `pin_models.py` pins them and checks their tensor names where Hugging Face is reachable, and until then they install only from supplied files.

The WordLlama vectors come from Llama 2 and Llama 3 token embeddings; their Llama Community License terms need review before Beans ships or recommends them.

### Pins

A file is pinned when the catalog gives its exact size and SHA-256. Only a model whose every file is pinned downloads; any model installs from files the user supplied. `wordllama-l2-256` is pinned from the `wordllama==0.4.0.post1` PyPI wheel, which ships the same two files; the others are not pinned yet.

`scripts/schedule-intent-eval/pin_models.py` pins them where Hugging Face is reachable: it resolves the revision to a commit, downloads each file, checks it against the SHA-256 Hugging Face lists for LFS files, checks an ONNX model's tensors against the entry with onnxruntime (both towers of an image model), refuses an empty file or a model over 1 GB, and writes the sizes, hashes, and commit back. It changes nothing for a model that does not read cleanly. `--offline DIR` pins from files already downloaded into `DIR/<repo>/<path>`.

## Installing

Installs reuse the local embedding installer (`crates/cli/src/embeddings/assets.rs`) and the memory setup rules:

1. `schedule_models.preview` freezes exactly which files, from where, of what size and SHA-256. It does no network I/O. Supplied files are hashed locally (at most 1 GB each, 256 MB for the runtime) and must match the pins of a pinned model. An ONNX or image model preview without `runtime` takes the one shipped beside the CLI, else the pinned wheel for this platform (an `archive` source: the wheel's URL, size, SHA-256, and the library's path inside it; the asset's own size and SHA-256 are the library's), shown in `assets` like the model's own files and summarized in `runtime_download`.
2. The response carries a one-use `preview_token` and `preview_digest`, valid for 10 minutes, bound to the account and the requesting Device.
3. `schedule_models.apply` with `confirm: true` spends the approval whatever happens next, then stages each file into a private directory, checks size and SHA-256 while writing, and renames the directory into place. A file that differs installs nothing.
4. Approvals and loads are dropped when the account pauses or is forgotten; a Device no longer paired, another account, or a draining Runner is refused at every step.

Downloads go only to the model hub (`schedule_models/hub.rs`), the one place that knows its address: `https://huggingface.co` by default. Because Hugging Face answers a file URL with a redirect to its CDN, the installer follows at most three redirects, each only to `huggingface.co`, `hf.co`, or their subdomains over HTTPS. `BEANS_MODEL_HUB` names another hub with the same `<repo>/resolve/<revision>/<path>` layout: a mirror over HTTPS, or, over plain HTTP, only one on this machine (tests and development), whose downloads may redirect only to its own host. An invalid value falls back to Hugging Face with a warning. Authentication for gated repositories would be added in `hub.rs`. Listings and previews name the hub in use. The pinned size and SHA-256 still decide what is installed. Local memory embedding installs keep refusing every redirect.

## Suggestions

Models never ship with the app and never download on their own. `schedule_models.list` suggests one text and one image model for this Runner (`schedule_models/recommend.rs`, pure and tested rule by rule). The desktop Search pane lists the suggestion first, and a download starts only from the user's approval of a preview.

The Runner reads its computer each time it lists (`machine.rs`): total memory, free disk where models are kept, cores, and whether ONNX Runtime can run (a build with `embedding-local`, not a static musl build, and a runtime beside the binary or pinned to download; a download's library counts toward the disk a model needs). A fact the system does not give stays unknown and never rules a model out. From the 300 most recent messages it reads whether the chats need a multilingual model (over 5% of letters outside the Latin script, or common Spanish, Portuguese, French, German, or Italian words more than half as often as English ones) and whether they carry pictures.

A model fits when ONNX and image models have a runtime, it takes at most a quarter of memory once loaded (a static table about its size plus a fifth, a transformer 2.5 times its size plus 150 MB for the runtime), its download leaves 1 GB of disk free, and it is pinned or already installed. Each model's `fit` lists why it does not fit (`needs_onnx_runtime`, `not_enough_memory`, `not_enough_disk`, `not_downloadable`), then notes (`english_only`, `multilingual`, `slow_here`, `installed`). Among models that fit, the highest score wins, the catalog order breaking ties: `rank` × 10, +25 for a multilingual model or −30 for an English-only one when the chats need several languages, −5 for a transformer on fewer than 4 cores, +3 when installed. An image model is suggested only when the chats carry pictures.

Files live under the Runner's home in `schedule-models/assets/<account namespace>/<install id>/`, with `model.safetensors` or `model.onnx` (and an image model's `image_model.onnx`), `tokenizer.json`, and for ONNX the runtime library. `schedule-models/state.json` (mode 0600) keeps the account, mode, selection, and installs; another account's state reads as nothing saved. Reinstalling a model replaces its files; `schedule_models.remove` deletes them.

## Loading and the classifier

Loading checks every file's size and SHA-256 again, builds the encoder, and, the first time for a given set of files, trains a logistic regression over the model's sentence vectors from `training.json`: 2,831 template messages in nine languages, requests to repeat something against one-off requests, bare tasks, habits, and questions. The training is deterministic (full-batch Adam, class-balanced, scikit-learn's `C = 3`), and the weights are kept in `state.json` with a digest of the training set, so a changed training set retrains.

A message is read whole and clause by clause (split at `, . ; : ! ?` and their CJK forms, at most 48 parts), and its score is its best part's.

`held_out.json` holds 40 hand-written messages (20 requests, 20 look-alikes) no model trains on. On it, `wordllama-l2-256` finds 17 requests and flags 3 look-alikes, missing two German requests and a Chinese one and flagging a French and two Japanese habits; the Rust Runner and the Python harness (`scripts/schedule-intent-eval/evaluate.py`) agree exactly. The threshold was chosen on those same messages, so expect somewhat lower on new ones.

## Methods

All go to the Runner named by `runner_id` (through a signed request when it is another Device, waiting up to 15 minutes), from the user's apps only; no model tool reaches them. Errors are short codes.

| Method | Params | Result |
| --- | --- | --- |
| `schedule_models.list` | `runner_id` | `models` (each catalog entry with `pinned`, `bytes`, `needs_onnx_runtime`, `downloads` `[{role, url, path, bytes, sha256}]`, `installed`, `selected`, `fit` `{fits, reasons, memory_bytes}`), `recommended` `{text, image}`, `machine` `{memory_bytes, free_disk_bytes, cores, os, arch, onnx_runtime}`, `needs` `{multilingual, images}`, `mode`, `selected`, `max_bytes`, `runs_models`, `onnx_runtime`, `note` |
| `schedule_models.status` | `runner_id` | `mode`, `selected`, `installed` (ids), `state` (`ready`, `loading`, `not_loaded`, `failed`, `none_selected`, `runtime_unavailable`), `error` |
| `schedule_models.preview` | `runner_id`, `model_id`, `source` (`{kind:"download"}` or `{kind:"supplied", files:{weights|model, image_model?, tokenizer}}` with absolute paths), `runtime?` (absolute path, ONNX and image models; default the shipped one) | `preview_token`, `preview_digest`, `expires_in_seconds`, `runner_id`, `model_id`, `name`, `license`, `pinned`, `source`, `hub` (null for supplied files), `runtime_download` (`{version, bytes, from}` when the runtime downloads with the model), `total_bytes`, `assets` |
| `schedule_models.apply` | `runner_id`, `preview_token`, `preview_digest`, `confirm` | `installed`, `model_id`, `state` |
| `schedule_models.select` | `runner_id`, `mode`, `model_id` (required for `local` and `local_then_reader`) | `mode`, `selected`, `state` |
| `schedule_models.remove` | `runner_id`, `model_id` | `removed`, `mode`, `selected` |

Preview errors include `unknown_model`, `model_not_pinned`, `file_required`, `unknown_file_role`, `absolute_path_required`, `file_unavailable`, `asset_mismatch`, `model_too_large`, `runtime_required`, and `runtime_not_needed`. Apply errors include `confirmation_required`, `approval_not_found`, `approval_expired`, `approval_stale`, and `asset_install_failed`. Load states and errors include `files_missing`, `files_changed`, `runtime_unavailable`, `model_contract`, and `training_failed`.

## Going live

Everything up to the real hub is built and tested with mocks. Where Hugging Face is reachable:

1. `pnpm run models:pin` (needs `onnx` and `onnxruntime` in Python for the tensor checks; `HF_TOKEN` for a gated repository) pins every unpinned entry: the multilingual text models and the image model. Review the diff, and each license, before committing it.
2. `pnpm run test:rust --live-hub` checks that the hub serves every pinned file, and every pinned runtime wheel, at its pinned size (asking for one byte each), and downloads, installs, and measures `wordllama-l2-256` end to end. Run it again after pinning, and whenever a download fails for users.
3. A mirror or a proxy is `BEANS_MODEL_HUB`. A hub that needs credentials gets them in `hub.rs`, the one place that builds download URLs.

## Semantic search

Installed models also serve [semantic search](semantic-search.md): text models embed messages, image models pictures. `search_index.select` chooses among installs and never installs anything itself; removing an install leaves search unable to load it. Applying an image model answers `state: "installed"` (no classifier), and `schedule_models.select` refuses one (`image_model_reads_no_schedules`).

## Builds

The catalog, API, routing, and installs build with `runner`. Running a model needs the `embedding-local` feature (`tokenizers`, `image`, and `ort` with a dynamically loaded runtime), which the `cli` feature and so every release build include. A build without it still installs models and reports `runtime_unavailable`. The Linux desktop app's CLI links glibc 2.28 (the oldest the pinned wheels support) so it can load the runtime shipped beside it. A static musl CLI (the standalone Linux CLI releases) cannot load a shared library: it suggests and runs static models only, and says ONNX and image models need ONNX Runtime.

## Tests

`cargo test -p beans --lib schedule_models` covers the catalog rules, the installer (exact kinds, redirects only to named hosts), preview, consent, one-use approvals, requester and account checks, supplied and changed files, selection, removal, and the turn's modes, against a fixture catalog so they hold whichever shipped models are pinned. With `--features embedding-local` they also load a fixture static model and train its classifier.

Hugging Face is never reached in tests. `schedule_models/mock_hub.rs` serves the catalog's layout from an origin on `127.0.0.1` that redirects to a "CDN" on `localhost`, and logs every request; each test points the Runner at it (`set_hub`, as `BEANS_MODEL_HUB` does) and sets the computer it runs on (`set_machine`). They cover the whole path from suggestion to approval to search, nothing fetched before approval, and every failure a real hub can give (404, 500, 429, a truncated or longer body, wrong or empty bytes, a redirect to a foreign host or in a loop, a hub that is down), each installing nothing and keeping the install already there. `scripts/schedule-intent-eval/test_pin_models.py` runs `pin_models.py` against a mock of the Hugging Face revision API and file downloads (`pnpm run test:rust` runs it). The runtime download is tested the same way: a wheel served by the mock hub, refused when it is another wheel, cut short, missing, or pinned without the library, and with the real runtime zipped (`onnx_runtime_downloads_its_runtime_when_none_ships_beside_the_cli`) loading an image model, then a second model with its own copy in the same process. Three tests run with `--ignored` given real files:

- `BEANS_SCHEDULE_MODEL_DIR` (the two `wordllama-l2-256` files): the held-out accuracy above, and a download of the pinned files through a local origin that redirects to a local CDN, with a changed file refused.
- `BEANS_SCHEDULE_ONNX_DIR` (`model.onnx` with all-MiniLM-L6-v2's tensors, `tokenizer.json`, and an ONNX Runtime library): install, native load, training, and reading through the supplied runtime.
