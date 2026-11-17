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

`catalog.json` is built in. Each model is at most 100 MB with its tokenizer (`MAX_MODEL_BYTES`), runs on one CPU thread, and names its Hugging Face repository, revision, files, license, languages, and engine:

- `static`: a token embedding table (safetensors) and its tokenizer. A text's vector is the unit-length mean of its tokens' rows. No native runtime.
- `onnx`: a transformer through ONNX Runtime, with the same pinned contract as local memory embeddings (tensor names, pooling, padding, maximum tokens). The user supplies the ONNX Runtime library (1.27 or newer, CPU build) as a file.

| Id | Engine | Size | Languages |
| --- | --- | --- | --- |
| `wordllama-l2-256` | static | 18 MB | English and other Latin-script languages; weak in Chinese and Japanese |
| `wordllama-l3-128` | static | ~42 MB | Llama 3 tokenizer; trained on English |
| `wordllama-l3-256` | static | ~75 MB | Llama 3 tokenizer; trained on English |
| `potion-base-8m` | static | ~32 MB | English |
| `all-minilm-l6-v2-int8` | onnx | ~24 MB | English |
| `bge-small-en-v1.5-int8` | onnx | ~35 MB | English |

Multilingual transformer embedders (`multilingual-e5-small`, `paraphrase-multilingual-MiniLM-L12-v2`) are left out: their 250,000-token vocabularies put them above 100 MB even at 8 bits.

The WordLlama vectors come from Llama 2 and Llama 3 token embeddings; their Llama Community License terms need review before Beans ships or recommends them.

### Pins

A file is pinned when the catalog gives its exact size and SHA-256. Only a model whose every file is pinned downloads; any model installs from files the user supplied. `wordllama-l2-256` is pinned from the `wordllama==0.4.0.post1` PyPI wheel, which ships the same two files; the others are not pinned yet.

`scripts/schedule-intent-eval/pin_models.py` pins them where Hugging Face is reachable: it resolves the revision to a commit, downloads each file, checks it against the SHA-256 Hugging Face lists for LFS files, checks an ONNX model's tensors against the entry with onnxruntime, refuses anything over 100 MB, and writes the sizes, hashes, and commit back. It changes nothing for a model that does not read cleanly. `--offline DIR` pins from files already downloaded into `DIR/<repo>/<path>`.

## Installing

Installs reuse the local embedding installer (`crates/cli/src/embeddings/assets.rs`) and the memory setup rules:

1. `schedule_models.preview` freezes exactly which files, from where, of what size and SHA-256. It does no network I/O. Supplied files are hashed locally (at most 100 MB each, 256 MB for the runtime) and must match the pins of a pinned model.
2. The response carries a one-use `preview_token` and `preview_digest`, valid for 10 minutes, bound to the account and the requesting Device.
3. `schedule_models.apply` with `confirm: true` spends the approval whatever happens next, then stages each file into a private directory, checks size and SHA-256 while writing, and renames the directory into place. A file that differs installs nothing.
4. Approvals and loads are dropped when the account pauses or is forgotten; a Device no longer paired, another account, or a draining Runner is refused at every step.

Downloads go only to `https://huggingface.co`. Because Hugging Face answers a file URL with a redirect to its CDN, the schedule-model installer follows at most three redirects, each only to `huggingface.co`, `hf.co`, or their subdomains over HTTPS. The pinned size and SHA-256 still decide what is installed. Local memory embedding installs keep refusing every redirect.

Files live under the Runner's home in `schedule-models/assets/<account namespace>/<install id>/`, with `model.safetensors` or `model.onnx`, `tokenizer.json`, and for ONNX the runtime library. `schedule-models/state.json` (mode 0600) keeps the account, mode, selection, and installs; another account's state reads as nothing saved. Reinstalling a model replaces its files; `schedule_models.remove` deletes them.

## Loading and the classifier

Loading checks every file's size and SHA-256 again, builds the encoder, and, the first time for a given set of files, trains a logistic regression over the model's sentence vectors from `training.json`: 2,831 template messages in nine languages, requests to repeat something against one-off requests, bare tasks, habits, and questions. The training is deterministic (full-batch Adam, class-balanced, scikit-learn's `C = 3`), and the weights are kept in `state.json` with a digest of the training set, so a changed training set retrains.

A message is read whole and clause by clause (split at `, . ; : ! ?` and their CJK forms, at most 48 parts), and its score is its best part's.

`held_out.json` holds 40 hand-written messages (20 requests, 20 look-alikes) no model trains on. On it, `wordllama-l2-256` finds 17 requests and flags 3 look-alikes, missing two German requests and a Chinese one and flagging a French and two Japanese habits; the Rust Runner and the Python harness (`scripts/schedule-intent-eval/evaluate.py`) agree exactly. The threshold was chosen on those same messages, so expect somewhat lower on new ones.

## Methods

All go to the Runner named by `runner_id` (through a signed request when it is another Device, waiting up to 15 minutes), from the user's apps only; no model tool reaches them. Errors are short codes.

| Method | Params | Result |
| --- | --- | --- |
| `schedule_models.list` | `runner_id` | `models` (each catalog entry with `pinned`, `bytes`, `needs_onnx_runtime`, `downloads` `[{role, url, path, bytes, sha256}]`, `installed`, `selected`), `mode`, `selected`, `max_bytes`, `runs_models`, `onnx_runtime`, `note` |
| `schedule_models.status` | `runner_id` | `mode`, `selected`, `installed` (ids), `state` (`ready`, `loading`, `not_loaded`, `failed`, `none_selected`, `runtime_unavailable`), `error` |
| `schedule_models.preview` | `runner_id`, `model_id`, `source` (`{kind:"download"}` or `{kind:"supplied", files:{weights|model, tokenizer}}` with absolute paths), `runtime` (absolute path, ONNX only) | `preview_token`, `preview_digest`, `expires_in_seconds`, `runner_id`, `model_id`, `name`, `license`, `pinned`, `source`, `total_bytes`, `assets` |
| `schedule_models.apply` | `runner_id`, `preview_token`, `preview_digest`, `confirm` | `installed`, `model_id`, `state` |
| `schedule_models.select` | `runner_id`, `mode`, `model_id` (required for `local` and `local_then_reader`) | `mode`, `selected`, `state` |
| `schedule_models.remove` | `runner_id`, `model_id` | `removed`, `mode`, `selected` |

Preview errors include `unknown_model`, `model_not_pinned`, `file_required`, `unknown_file_role`, `absolute_path_required`, `file_unavailable`, `asset_mismatch`, `model_too_large`, `runtime_required`, and `runtime_not_needed`. Apply errors include `confirmation_required`, `approval_not_found`, `approval_expired`, `approval_stale`, and `asset_install_failed`. Load states and errors include `files_missing`, `files_changed`, `runtime_unavailable`, `model_contract`, and `training_failed`.

## Builds

The catalog, API, routing, and installs build with `runner`. Running a model needs the `embedding-local` feature (`tokenizers`, and `ort` with a dynamically loaded runtime), which default builds leave off; without it a Runner still installs models and reports `runtime_unavailable`, and local modes read nothing.

## Tests

`cargo test -p beans --lib schedule_models` covers the catalog rules, the installer (exact kinds, redirects only to named hosts), preview, consent, one-use approvals, requester and account checks, supplied and changed files, selection, removal, and the turn's modes, against a fixture catalog so they hold whichever shipped models are pinned. With `--features embedding-local` they also load a fixture static model and train its classifier. Three tests run with `--ignored` given real files:

- `BEANS_SCHEDULE_MODEL_DIR` (the two `wordllama-l2-256` files): the held-out accuracy above, and a download of the pinned files through a local origin that redirects to a local CDN, with a changed file refused.
- `BEANS_SCHEDULE_ONNX_DIR` (`model.onnx` with all-MiniLM-L6-v2's tensors, `tokenizer.json`, and an ONNX Runtime library): install, native load, training, and reading through the supplied runtime.
