# Phone memory control assessment

Issue [#103](https://github.com/bloodf/beans/issues/103), PR [#356](https://github.com/bloodf/beans/pull/356). Source baseline is main `f3f793d49f40e66f416084c512aba2738a3f867e`, integrated into the reserved PR without rewriting phone pages. This is a source and fixture contract, dated 2026-11-07. Satisfied means the source implements the control; it does not mean device acceptance passed.

## AppKit checklist mapping

The acceptance list in [#101](https://github.com/bloodf/beans/issues/101) is the comparison contract. References below name current definitions or page controls so they remain searchable as line numbers change.

| Control | Assessment | Source evidence and remaining acceptance |
| --- | --- | --- |
| Create connection with keep/replace/clear | satisfied | `mobile/app/settings/memory/connection.tsx` calls `setConnection` with `secretPatch`; `mobile/src/ui/memoryUI.tsx` exposes `MemorySecret`; `mobile/src/core/memoryService.ts` `connectionRequest` preserves omitted fields and strips arbitrary scope. Existing `memoryService.test.ts` covers masked configuration, secret patches and blocked Lance Cloud configuration. Device save/reopen is deferred. |
| Bind bot and enable recall with consent | satisfied | `mobile/app/chat-info/memory/[id].tsx` connection chooser, Approve Exact Recall/Capture Choices and Save use `MemoryPreferencesDraft` and `savePreferences`. Draft defaults disable capture/recall; exact bot/connection/choices approval is required. `memoryService.test.ts` covers approval; `memoryAdvanced.test.ts` covers cap increases and invalid budgets. |
| Changing connection clears approvals | satisfied | `MemoryPreferencesDraft.connectionID` clears plaintext/group approvals and invalidates saved consent, including switching back. Existing consent fixture in `memoryService.test.ts` exercises connection changes. |
| Missing capability disables rather than hides | partial | `[id].tsx` `reason` supplies disabled reasons to `MemoryAction` for basic controls. `memoryUI.tsx` removes `onPress` when a reason exists. Advanced panels use `advanced.filter` and `advancedForms`, so absent feature/verb hides them. This is an explicit difference from #101, consistent with the current shared advanced-action contract in `docs/architecture/memory-ui.md`. No new issue or page rewrite is proposed without a product decision to change that contract. `phoneMemoryContract.test.ts` proves each basic action rejects absent/false capability before dispatch and sends the exact payload only when enabled. `memoryAdvanced.test.ts` covers absent maps and sibling verbs. |
| Health, delivery_unknown and deletion pending are distinct | satisfied | `[id].tsx` displays Service state, Readiness reason, Deletion pending, each operation state and the queue deletion state separately. `readMemoryHealth` and `readMemoryOperations` retain these states. Only completed means stored; pending never means verified erasure. Existing service and deletion fixtures cover parsing and deletion refresh locks. Device presentation remains deferred. |
| Every control has an accessible name | missing | `mobile/src/ui/forms.tsx` `ToggleRow` and `FieldRow` do not attach their sibling title/label to Switch/TextInput in this baseline. Already assigned [#329](https://github.com/bloodf/beans/issues/329), PR #362. No duplicate issue or shared-form edit here. VoiceOver/TalkBack acceptance is NOT_RUN. |

## Other shared controls

| Control | Assessment | Source evidence and boundary |
| --- | --- | --- |
| Embedding profiles | satisfied | `mobile/app/settings/memory/embedding.tsx` supplies full API/local profile edits; `embeddingRequest` validates vector space, preserves prefixes and excludes local Runner paths. `memoryAdvanced.test.ts` covers that boundary. Saving does not perform inference or reindexing. |
| English and Simplified Chinese | missing | Memory page text and `memoryUI.tsx` disclosures are English literals at this baseline. Already assigned [#330](https://github.com/bloodf/beans/issues/330), PR #365. This assessment does not edit localization. |
| Explicit service action and no automatic remote work | satisfied | `[id].tsx` opens with listConnections/getPreferences only. Health/retain/reflect use explicit confirmations. `memoryServiceNative.ts` passes the existing public native `request` to the adapter. Fixture dispatch proves adapter behavior, not native transport delivery or mounted UI behavior. |
| Fenced deletion and setup | satisfied | `memoryTask.ts` `MemoryDeletionState` locks and refreshes authoritative preferences after deletion. `memoryDeletion.test.ts` tests failure and epoch handling. `memoryService.ts` setup approval objects bind target/revision/expiry and are consumed once; `memoryAdvanced.test.ts` tests stale, expired and reused approvals. No remote erasure or DDL is executed by these fixtures. |
| Local MEMORY.md | satisfied | `mobile/app/chat-info/memory/local.tsx` uses the separate local editor with expected hash; `memoryAdvanced.test.ts` tests stale-hash refusal. This is independent of service capture. |

## Verification and acceptance boundary

Run the existing mobile Vitest configuration with `src/core/phoneMemoryContract.test.ts`, `src/core/memoryService.test.ts`, `src/core/memoryAdvanced.test.ts` and `src/ui/memoryDeletion.test.ts`. Run `pnpm run check:docs` and the existing mobile typecheck through the assigned executor. There is no mobile lint script in this baseline. This change adds fixtures and documentation only; it changes no native build input or production mechanism.

Manual device, hardware, browser, E2E and full-suite acceptance are NOT_RUN/deferred. In particular #103's manual scenario remains open. #329/#330 completion must be reconciled at their actual merged heads, followed by device acceptance; draft ownership is not evidence that either gap is fixed. No additional confirmed missing implementation outside those existing assignments was found in this bounded assessment.
