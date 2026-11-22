# Semantic search

Exact search (`chats.search`, see [macOS sidebar](macos-sidebar.md)) finds the words typed. Semantic search finds messages by meaning ("canine at the beach" finds "my puppy loves the shore") and pictures by what they show. A Runner does the work. The desktop palette and the phone's chat search show **Related** and **Pictures** sections under the exact results, whose contract is unchanged.

## Choices

Settings › Search (per Runner and account; `search_index.select`) holds three independent choices:

| Role | Choice | Leaves the Runner |
| --- | --- | --- |
| Text | An installed local catalog model, or an API-mode embedding profile (`memory.embeddings.set`) | With a profile: every indexed message goes to its provider |
| Images | An installed local image model (engine `clip`): its vision tower embeds pictures, its text tower embeds queries into the same space | Nothing |
| Describer | A connected provider's model that takes images: it writes what each picture shows, which the text model then embeds | Every picture, once; tokens count in the chat's spending |

Choosing a provider or a describer needs `confirm_external: true`, which the apps ask for with a warning; re-selecting the same choice does not ask again. Models come from the [schedule-models](schedule-models.md) catalog and install through its one-use, consent-bound preview and apply; search only chooses among installs. Nothing downloads on its own: the pane lists the model the Runner suggests for its computer and chats first, says why others do not fit, and downloads only after the user approves a preview (see [Suggestions](schedule-models.md#suggestions)). The catalog has multilingual text models (Chinese and Japanese included) and an image model whose text tower reads English. Pictures are found in other languages through descriptions.

## What is embedded

The text role embeds the visible text exact search reads (message text, handoff reasons, notices, permission summaries), each attachment's name, and each picture's description. The image role embeds picture attachments already on the Runner, prepared as the agent prepares images (`files::inline_image`: HEIC converted on a Mac, rotated upright). Indexing never fetches from the relay; a picture that arrives later (`files::ensure_local`) queues its message again. A picture that does not decode is skipped, not retried forever. Tool output, hidden thinking, and chats outside the roster are never read. A message is embedded once it is finished (complete, or failed with partial words).

Text is whitespace-normalized and cut into chunks of at most 800 characters, ending at a space in the chunk's last quarter and repeating about 120 characters of the one before. Text without spaces is cut by characters. A message gives at most 32 text chunks and 16 attachments. Chunk numbers are deterministic per kind (text from 0, names from `1<<20`, descriptions from `2<<20`, pictures from `3<<20`, by attachment index), so a hit names its snippet and attachment.

## Storage and freshness

Vectors live in the Runner's store (`local_store/vectors.rs`) as int8 codes and one scale per chunk, tagged with a role and a space. The space is a hash of the chunker, the model, and its exact files, or the profile's embedding fingerprint. Each role keeps one space (`message_vector_spaces`). Choosing another model, reinstalling with other files, or changing the chunker drops that role's vectors and queues everything again. A provider text source is sent text through the secret shield ([Secrets](secrets.md)), documents and queries alike, so a secret's value becomes `{{secret:NAME}}` before it leaves; its space id includes `shielded`, so an index built before the shield is rebuilt once. Vectors are sensitive derived data: they never sync or reach the relay, and are cleared with the account.

Work queues are flags on the exact-search projection (`message_search`): `embed_pending`, `image_pending`, and `describe_pending`, each read newest first from its own partial index, so a pass costs its batch and not the backlog. Triggers (`sql/schema/search.sql`) queue a message and drop the matching vectors at once whenever its text, names, or images change, and delete everything with the message. This holds for writes from any binary. Each change also bumps the queue's version (`embed_version`, `image_version`). Work saves only if the version it read still holds and its space is still chosen, so vectors never outlive their text, images, or model, and a description arriving mid-embed is not lost. Catching the projection up after writes runs in batches of 512 rows, releasing the store between them.

Vectors are scaled to unit length before they are quantized, so every score is a cosine whatever the model or provider returns.

The indexer runs for the process's life on a Runner: up to 32 messages of text, 8 of pictures, and 4 of descriptions per pass, 25 ms apart while work remains, otherwise every 5 seconds. It does nothing while the account is paused. History embeds gradually after a model is chosen. A queue whose model fails waits before it tries again, 5 seconds doubling to 10 minutes, while the other queues go on; a describer that fails partway keeps the descriptions it wrote and leaves the message queued.

## Queries

`chats.search_semantic` embeds the query (at most 1000 characters) once per ready role and scans that space's in-memory copy exactly, by brute force, on int8 dot products: measured at about 75 ms and 384 MB per million 384-dimension chunks on one core. The copy reads rows past the highest id it holds (ids are `AUTOINCREMENT`) and applies the deletion log a trigger fills, compacting once a quarter of its rows are deleted.

The copy only proposes candidates. Each is confirmed against current rows (still stored, the message current and not queued again) and the roster before it is returned, so a stale copy costs recall, never correctness. Results are one per message, best first, and drop below half the best confirmed score or the caller's `min_score`: a stale best candidate never sets the floor. A role whose provider fails reports `failed` with the error; the other role still answers. Clients hide hits an exact section shows and show each picture once.

A Device that is not a Runner, such as the phone, asks one through the relay: the `runner_id` given, otherwise each paired Runner in turn, online ones first, until one has a ready space: 10 seconds for each, 20 in all. It keeps only hits in chats it has, and asks only after the exact results arrive.

## Runtime

Release builds include the `embedding-local` feature. The macOS app and the Windows desktop app ship the pinned ONNX Runtime beside the CLI (`scripts/onnxruntime.ts`, `crates/cli/onnxruntime.json`: official PyPI wheels by size and SHA-256). Previews use it when no runtime is named, and show it for approval like the model's files; a Runner with none beside it (a CLI installed on its own) downloads the same pinned wheel with the model's approved files. Models are never bundled and never download without the user's approval. The Linux desktop app ships a glibc CLI with the runtime. The standalone Linux CLI is static (musl) and cannot load a shared library, so it runs static catalog models only, and suggests only those.

## Limits

- Brute force suits up to about a million chunks per space. Larger stores need an approximate index (LanceDB is linked for Runner memory) behind the same candidate-then-confirm contract, and exact search would want a trigram index.
- The shipped image model's text tower reads English; a multilingual image model needs a verified ONNX export.
- New catalog entries stay unpinned until `pin_models.py` reads them where Hugging Face is reachable, which checks their tensors too.

## API

| Method | Params | Result |
| --- | --- | --- |
| `chats.search_semantic` | `query`, `limit?` (1–50, default 20), `min_score?`, `runner_id?` | `state`, `related`, `image_state`, `images`, `runner_id`, `error` |
| `search_index.status` | `runner_id?` | `text`, `image` (`choice`, `state`, `error`, `indexed`, `pending`), `describe` (`choice`, `pending`), `profiles`, `runs_models` |
| `search_index.select` | `runner_id?`, any of `text` (`{kind:"local",model_id}`, `{kind:"provider",profile_id}`, or null), `image_model_id` (or null), `describe` (`{provider,model}` or null), `confirm_external?` | as `status` |

A state is `ready`, or says why its list is empty: `off`, `loading`, `failed`, `runner_required`, `no_runner`, or `unreachable`. A hit has `kind` (`message`, `file`, or `image`), `chat_id`, `message_id`, `snippet`, `author`, `created_at` and `score`; file and image hits add `attachment_id` and `name`. Select errors include `model_not_installed`, `not_a_text_model`, `not_an_image_model`, `profile_not_found`, `profile_not_api`, `model_without_images`, and `external_confirmation_required`. `search_index.*` checks the requester is this Device or still paired.

## Tests

See [Testing](testing.md). `search_index::tests` runs fixture models that behave like real ones where it matters (synonyms share a dimension, an image tower sees colour, a scripted vision model describes), through choice, queues, edits, deletions, races, providers, and confirmation. `onnx_runtime_finds_pictures_with_an_installed_image_model` installs a tiny image model through the real preview and runs it on the real ONNX Runtime.
