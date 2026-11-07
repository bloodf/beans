# Prepare and use AI tooling

Read [AGENTS.md](../AGENTS.md), [BEANS.md](../BEANS.md) and [ARCHITECTURE.md](../ARCHITECTURE.md) before changing Beans. `CLAUDE.md` points to `AGENTS.md`; preserve that link and all existing instructions.

## Keep product work on HOLD

This preparation permits onboarding, tooling installation, independently reviewed tooling PRs, normal tooling merges and branch propagation. Product work remains HOLD until Heitor explicitly says OK. Green checks, a tooling merge and completed propagation do not release product work. Do not merge product PRs during this rollout.

The central coordinator owns global settings and routing. Read `/Users/heitor/.config/ai-tooling/COORDINATION.md` and the current `GLOBAL-ROLLOUT.md` linked there. Report global configuration gaps to that coordinator. Keep one external rollout REPORT with commands, exit codes, receipt hashes, exact heads, resources and blockers.

Target `bloodf/beans` explicitly for GitHub operations. `origin` is the Beans repository; `upstream` is `egoist/lorca`. T3's upstream canonical identity does not authorize upstream publication or integration. Confirm remotes, branch, HEAD, current base, worktrees and dirty state before writing. Similar source content does not prove ancestry.

Preserve all refs with a verified Git bundle before propagation. Save staged and unstaged patches, untracked files and their hashes separately. Use an isolated tooling checkout. Preserve existing worktrees and previous work. Do not use automatic stash, `reset --hard`, force push or shared cleanup.

## Retrieve source with CodeGraph 1.6.2

The standalone CLI is pinned by `.codegraph-version`. `scripts/codegraph.sh` checks the installed version, disables telemetry for its process and runs from this checkout's root. Keep CodeGraph outside application dependencies.

Use the existing installation when it matches the pin. For workstation installation, inspect the installer at `https://raw.githubusercontent.com/colbymchenry/codegraph/f72c9d2efb14454ff4f9fd7b8460abd82f19a287/install.sh`, then run the inspected file with `CODEGRAPH_VERSION=v1.6.2`. Installation and user-specific MCP configuration belong to the central coordinator.

Run these commands from the repository root:

```sh
sh scripts/codegraph.sh init --yes
sh scripts/codegraph.sh sync
sh scripts/codegraph.sh status --json
sh scripts/codegraph.sh query dispatch --limit 5 --json
sh scripts/codegraph.sh callers dispatch --file crates/cli/src/api.rs --limit 5 --json
sh scripts/codegraph.sh explore --max-files 3 'api dispatch websocket serve'
```

Start with a bounded query before broad source reads. For exact text, missing results or unsupported syntax, use bounded `rg` and exact file ranges. Inspect the actual definition and caller before changing a flow. A graph relationship does not prove runtime behavior or complete dynamic call coverage.

Each worktree has its own `.codegraph/` index. Do not copy databases between checkouts. Run `sync` and inspect `status --json` after checkout or edits. Check indexed languages, file/node counts, pending changes, worktree mismatch and index completion. Rebuild with `index` after changing include/exclude/extension rules. Version 1.6.2 has no `--strict` flag.

Track only `.codegraph/.gitignore` inside `.codegraph/`. The ignore file excludes databases, logs, sockets, locks and daemon state. Never commit generated graphs or index data. With a configured CodeGraph MCP, keep the default single `codegraph_explore` tool; otherwise use the same bounded CLI commands.

## Preserve command status with RTK 1.0.1

Read `/Users/heitor/.codex/RTK.md` when present. With a native RTK PreToolUse hook, run normal commands and do not wrap them twice. Without a native hook, use supported explicit commands such as `rtk git status` and `rtk gh pr list --repo bloodf/beans`. MiniMax uses this CLI route. Do not assume one harness's hooks apply to another.

Preserve the actual exit code and first failure receipt. If condensed output is empty, contradictory or unusable, recover it with `rtk bypass <cmd>` or the stored raw receipt. Diagnose the first failure before another attempt. Do not weaken a check, timeout, baseline, security boundary or review requirement to obtain green results.

## Use Caveman and Ponytail

Use concise, complete Portuguese sentences with the user. Keep errors, identifiers, numbers and negations exact. Write code, persisted documents and critical instructions in complete language.

Inspect the affected flow and callers, reuse existing code and standard libraries, and make the smallest complete change. Verify behavior at the relevant boundary. Preserve security, accessibility, approval, encryption and data-loss controls. Installation, synthetic probes, local checks, hosted checks and live behavior are separate evidence.

## Verify TokenEconomizer per harness

Reuse the central installation. Preserve the mode without auxiliary paid model calls: `microskills=envfacts`, `loopguard=off`, `repomap=off` and `toolprune=propose`. Do not set a paid model or enable paid proposers. Keep orchestration, review, device and required verification tools available; proposals cannot override project policy.

From the checkout, inspect effective state without a model call:

```sh
"${TOKENECONOMIZER_HOME:-$HOME/.tokeneconomizer}/bin/te" --host codex --cwd "$PWD" status
"${TOKENECONOMIZER_HOME:-$HOME/.tokeneconomizer}/bin/te" --host codex --cwd "$PWD" config show
"${TOKENECONOMIZER_HOME:-$HOME/.tokeneconomizer}/bin/te" --host codex --cwd "$PWD" repos list
```

Confirm the repository identity is the Beans origin and verify a worktree maps to that same repository. Record the installed adapter's supported events for each harness actually used. Codex's installed hook file declares `SessionStart`, `UserPromptSubmit`, `PostToolUse`, `Stop` and `SessionEnd`; declaration alone does not prove delivery through T3.

Separate configuration present from fresh-session runtime proof. Loading a skill, starting a process or seeing a zero-session status does not prove the full lifecycle. Record unproved events as UNKNOWN. Do not invent adapters or hook support for Antigravity, MiniMax or another harness. TokenEconomizer learns repository facts and proposes tool pruning; it is not a generic output compressor. No local token savings are claimed without measurement.

## Reserve every worker through the central dispatcher

Before each tooling worker, reserve a route:

```sh
t3-model-route reserve --project beans --purpose tooling --task '<bounded task>'
```

Launch only the exact returned provider, model and options through the applicable T3 delegation mechanism. Antigravity accepts Google models only. MiniMax M3.1 Flash requires `m:minimax:MiniMax-M3.1-Flash:v:thinking`. Quota UNKNOWN is not proof of capacity. Never substitute another project, provider or model, retry quota blindly, edit the global helper or implement a local round robin.

Give each worker exact scope and file ownership. State that concurrent work must be preserved. Record the lease, task/run, provider, model and terminal outcome. Release only the exact lease after terminal evidence. Start no top-level thread for a worker. Use a fresh independent reviewer for each review or re-check, with the exact base/head and applicable risk requirements. No author approves their own change.

## Verify and propagate tooling

Keep the existing Beans gates. `pnpm run check:docs`, `pnpm run check:catalog` and the same-repository PR test matrix remain applicable. Inspect the current workflow definitions before execution; a local tooling smoke does not replace hosted checks. Product homes, accounts, relays, signing inputs and deployments are not tooling fixtures.

Create one tooling PR against current Beans `main` and link it immediately to T3. Merge only after independent exact-head review and the real required checks pass. When waiting for hosted checks, use T3's PR watch and end the run; do not poll CI.

Before propagation, prepare a per-ref plan with original head, target payload, old-head ancestry proof and recovery instructions. Append only tooling to every existing local and origin branch, including historical branches and open PR heads. Preserve each branch's existing instruction and ignore bytes. Do not merge unrelated product history, silently resolve divergence or recreate a branch absent from the server. Push fast-forward with explicit refspecs. Stop on the first remote write failure, re-read remote state and report to central before retry or a strategy change.

Recheck current remote and PR heads, rescan for new refs, validate indexes in used checkouts and settle owned processes and leases. Record any uncovered ref as BLOCKED with a concrete reason. Product work still waits for Heitor's explicit OK.
