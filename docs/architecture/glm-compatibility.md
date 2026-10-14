# Zhipu GLM compatibility

## Chosen contract

GLM reuses an existing custom `chat-completions` connection with a standard API key, explicit regional root, text-only requests, and server-default thinking. The verified disposition is local wire compatibility for tool-free text, with reasoning-enabled tool continuation unsupported. No separate runtime provider, preset, dependency, native-client change, or paid/vendor inference call is required. Local evidence does not certify live account access, model quality, geographic availability, data residency, billing, or subscription entitlement.

## Primary vendor documentation

Reviewed primary documentation on 2026-10-14:

| Subject | Primary source | Contract |
| --- | --- | --- |
| Z.AI API/auth | [Introduction](https://docs.z.ai/api-reference/introduction) | Standard root `https://api.z.ai/api/paas/v4`; direct bearer API key |
| BigModel API/auth | [HTTP guide](https://docs.bigmodel.cn/cn/guide/develop/http/introduction.md) | Standard root `https://open.bigmodel.cn/api/paas/v4`; direct bearer API key; optional JWT alternative |
| Chat wire | [Chat Completions](https://docs.z.ai/api-reference/llm/chat-completion.md) | `/chat/completions`, SSE text/reasoning, indexed function calls, usage, terminal reasons |
| Model | [GLM-5.3](https://docs.z.ai/guides/llm/glm-5.3.md) | `glm-5.3`, text-only, 1,000,000-token context, 128K output, mandatory reasoning, `low/high/max` effort |
| Tool reasoning | [Thinking modes](https://docs.bigmodel.cn/cn/guide/capabilities/thinking-mode.md) | Replay unchanged assistant `reasoning_content` with tool results; Coding Plan preserves thinking by default |

Choose the account platform that issued the key and confirm model access there. These roots do not establish universal regional access, data residency, or interchangeable credentials. Beans neither selects another region nor falls back to another host. The chosen contract uses directly supported API-key authentication, not JWT generation or refresh.

Use the masked provider sheet or `--api-key-stdin`; keep keys out of URLs, prompts, transcripts, command arguments and evidence. The existing adapter sends `Authorization: Bearer <api_key>` to the explicit root plus `/chat/completions`. Existing no-redirect and bounded request/stream behavior applies.

## Existing connection and encrypted credential mapping

The existing `providers.connect_custom` path validates the protocol and URL, discovers models, and saves a staged credential copy before publishing it in memory. Failed discovery or persistence does not commit a connection. `CustomProvider` retains its custom identity, root, key, ordered model selection and saved metadata. The account's `credentials` blob is encrypted with the account DEK before relay sync; the relay receives ciphertext. Each bot's assigned Runner resolves this connection and owns inference and tool execution. No new provider-specific secret storage or sign-in is needed. See [Providers](providers.md) and [Protocols](protocols.md#cli--relay).

Vendor inference docs do not establish `/models` support or inference authentication. Only existing 404/405 unsupported-listing handling permits manual IDs. Other discovery failures preserve configuration and drafts. Model IDs remain opaque; a `glm` name grants no capabilities or gateway catalog rates. Save verified window/image/tool declarations explicitly or retain unknown metadata. Unknown costs are not free. The vendor's `128K` output notation is not converted into an invented exact token count.

Leave thinking unset for this contract. Beans then sends no speculative thinking parameter. GLM-5.3 cannot disable reasoning. Beans' generic `openai` thinking declaration offers `minimal`, `low`, `medium`, `high` and `xhigh`; GLM-5.3 accepts the overlapping `low` and `high`, but not `minimal`, `medium` or `xhigh`, and its `max` is absent from that declaration. Beans has a matching `openai-low-high-max` vocabulary, but the current custom Chat Completions format gate does not allow it. Different account-enabled models and thinking-disabled behavior need separate evidence, never a model-name heuristic.

## Other wires and subscription limits

These vendor-published alternatives map to existing Beans adapters, but are not fixture-certified GLM paths:

| API | Z.AI root | BigModel root | Existing Beans mapping |
| --- | --- | --- | --- |
| `responses` | `https://api.z.ai/api/v1` | `https://open.bigmodel.cn/api/v1` | bearer, append `/responses` |
| `messages` | `https://api.z.ai/api/anthropic` | `https://open.bigmodel.cn/api/anthropic` | `x-api-key`, append `/v1/messages` |

Sources: the model page, [BigModel Responses](https://docs.bigmodel.cn/cn/guide/develop/responses/introduction.md), and [BigModel Messages](https://docs.bigmodel.cn/cn/guide/develop/claude/introduction.md). Responses defaults `store=false`, terminates without `[DONE]`, and offers no vendor cancel API. Beans replays local history; this evaluation does not certify server-stored continuation. Messages compatibility does not establish GLM support for Anthropic thinking formats or server tools. Z.AI's GLM-5.3 page restricts accounts with current or expired Coding Plan subscriptions to Chat Completions; account entitlement remains separate from wire compatibility.

Coding Plan roots are `https://api.z.ai/api/coding/paas/v4` and `https://open.bigmodel.cn/api/coding/paas/v4`, not standard API defaults or fallback endpoints. Both [Z.AI](https://docs.z.ai/devpack/tool/others.md) and [BigModel](https://docs.bigmodel.cn/cn/coding-plan/tool/others.md) restrict subscription benefits to listed tools. Beans is not listed in the reviewed documentation. No Coding Plan entitlement or quota routing is certified; do not impersonate a listed client.

## Supported and unsupported disposition

Local fixtures verify the existing Chat Completions transport decodes `delta.content`, `delta.reasoning_content`, fragmented indexed function arguments, ordinary tool-result replay, usage and errors. HTTP/SSE errors and `sensitive`, `network_error`, `model_context_window_exceeded` remain failures, not successful completions. The existing agent loop does not dispatch calls from a failed reply.

**Reasoning-enabled tool continuation is unsupported.** The vendor requires unchanged `reasoning_content` replay with tool results. Beans displays streamed reasoning but its Chat Completions assistant conversion omits that field. Preserved thinking being off on the standard API does not remove the vendor's interleaved-tool requirement; GLM-5.3 always reasons. Decoding a function call is not proof of a compatible multi-step GLM agent loop. Use tool-free requests for the chosen text contract. Leaving tool metadata unknown defaults `supports_tools` to `true`, so it does not enforce tool-free use. Set an explicit Tools No declaration to reject tool-bearing bot turns before HTTP rather than silently removing tools.

Cancellation aborts the Runner's local request/stream. Local socket closure does not prove vendor inference stops or charges cease. Vendor `clear_thinking`, audio/video/file inputs, hosted search/retrieval, async inference and automatic JWT renewal are outside this contract.

## Evaluation evidence

Runtime tested revision: `ac4f24391358edb82b7dbd99c7d63b1d5bfc9053`. Runtime sources remain unchanged; a temporary executable supplies synthetic local HTTP fixtures. Fixture SHA-256: `da4e6792b4eedcdf98f03cf3bdb040856d2e058af02d52a2427038532f4dfb46`. The fixture is removed from the repository after use; private reproduction source and redacted receipt are retained for independent review.

`cargo run --locked -p beans-agent --example glm_compat_check` builds and exercises real `OpenAiCompatProvider`, not a copied parser or mocked provider. It exits 0. Running the same-session built executable independently also exits 0. Both use only synthetic credentials and local HTTP, with no vendor calls or account configuration mutation. Captured output:

```text
PASS streaming: reasoning, text, usage, terminal stop; bearer and explicit route verified
PASS tools: fragmented arguments and result replay; LIMIT interleaved reasoning_content absent on replay
PASS terminal error: sensitive, no successful completion
PASS terminal error: network_error, no successful completion
PASS terminal error: model_context_window_exceeded, no successful completion
PASS HTTP/SSE error: 401 Unauthorized, Invalid API key
PASS HTTP/SSE error: 429 Too Many Requests, Rate limit reached
PASS HTTP/SSE error: 200 OK, Invalid parameter
PASS cancellation: partial stream aborts, no Done, server observes closed transport; vendor billing not proven
```

The fixture verifies outgoing bearer/path/model/stream fields, omits speculative thinking/cache fields, assembles fragmented arguments, checks tool IDs on replay, observes absent reasoning replay, rejects successful completion on failures, and verifies cancellation closes the local transport. Retries are disabled for deterministic error fixtures; retry behavior is not evaluated. No permanent runtime change means no new regression suite or broad package tests are needed. Encryption and staged-save mapping are source-traced existing mechanisms, not exercised by the transport fixture.

The upstream [issue 14](https://github.com/egoist/lorca/issues/14) asks “Can you support GLM? or open for PR?” rather than reporting a reproducible transport failure. Reviewed public issue remains open, unassigned, with no development branch/PR. Its single reviewed timeline page contains only this evaluation issue's cross-reference. Unlinked/private PR activity remains unknown; no upstream patch is imported. This is an independent existing/unsupported disposition, not a before/after runtime fix.
