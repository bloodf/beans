# Shell input deadline — code handoff (2026-10-12)

Status: **ready code, not started.** No source owner lease is held here; Main must clear the original
owner before any edit to `crates/agent/src/tools/bash_session.rs`.

Scope: port of upstream `fc3656d2` (`bash_input: return from a command that keeps printing; time
limits on CI jobs`, #151), **only** the input deadline branch. Cancellation, stop, `background`
flag and `session_result` stay untouched.

## Blocking facts

- Upstream `fc3656d2` is **not an ancestor** of this branch (`git merge-base --is-ancestor` → no).
  Port is manual, line-by-line, from `git show fc3656d2`.
- Local `.omc/state`, `.omc/handoffs`, `.omc/artifacts` do **not** exist in this worktree, and no
  `shell297` / `shell-297` artifact is tracked in the repo. **The original shell297 lease evidence
  could not be located here.** Main owns that receipt; this handoff proceeds on the stated scope.
- PR creation is externally blocked; nothing in this handoff depends on it.

## Current state (verified at `bc284e80`)

Already present from earlier ports — do **not** re-add:

- `BACKGROUND_SETTLE = Duration::from_secs(2)` (`bash_session.rs:43`)
- `MAX_OUTPUT_WAIT = Duration::from_secs(300)` (`bash_session.rs:46`), used by `bash_output`
  (`bash_session.rs:1048`)
- `BashInputTool { sessions, waiting_after }` (`:938`), `BashInputTool::new` (`:945`)
- `bash_input` execute waits with **no deadline**:
  `session.wait(base, since, idle, None, &cancel, |_| {})` (`bash_session.rs:997`)

Callers of `BashInputTool::new`: `bash_session.rs:1125` (tests) only. `waiting_after()` builder used
in the same place. No production construction site outside the crate's tool registry — grep before
editing.

## Hunk 1 — production guard (crates/agent/src/tools/bash_session.rs)

1. Field on `BashInputTool` (`:938`), with the upstream doc comment:

   ```rust
   /// The longest the call waits on a command that keeps printing, so never goes quiet:
   /// `bash_output`'s longest wait.
   longest_wait: Duration,
   ```

2. `new` (`:945`): `BashInputTool { sessions, waiting_after: WAITING_AFTER, longest_wait: MAX_OUTPUT_WAIT }`

3. `description` (`:961`): insert into the first sentence, after "or goes quiet":
   `, or at most 2 seconds later for a command in the background and 5 minutes for any other`

4. `execute` (`:997`), replacing the single `wait` line:

   ```rust
   // A command that keeps printing never goes quiet, nor does one that took no notice of a
   // Ctrl-C: the call returns what it printed by then, with where it stands. One in the
   // background waits as long as starting it there does.
   let longest = if session.background() { BACKGROUND_SETTLE } else { self.longest_wait };
   let stop = session.wait(base, since, idle, Some(since + longest), &cancel, |_| {}).await;
   ```

`idle`, `cancel`, `base`, `since`, `session_result` unchanged. Deadline is a `Some(...)` only — no
fallback value, no new constant.

## Hunk 2 — regression test (same file, `mod tests`)

Local divergence from upstream parent: **no `printed()` helper exists here** (upstream has it at
parent line 1258). Add it before the new test, matching upstream signature
`async fn printed(session: &BashSession, text: &str)`.

New test, verbatim from `git show fc3656d2` (adds after `a_background_command_returns_while_it_prints`):

```rust
/// Typing into a command that keeps printing returns what it printed, never waiting for a
/// quiet spell that does not come: at once for one in the background, at the longest wait
/// otherwise. A Ctrl-C it takes no notice of reads the same.
#[cfg(any(unix, windows))]
#[tokio::test]
async fn input_returns_while_the_command_keeps_printing() {
    let mut t = tools(Duration::from_millis(400));
    t.input.longest_wait = Duration::from_secs(2);
    let ticks = "while true; do echo tick; sleep 0.1; done";

    let background = call(&t.bash, json!({"command": ticks, "background": true})).await.unwrap();
    let id = session_id(&background);
    let started = Instant::now();
    let typed = call(&t.input, json!({"session_id": id, "text": "x"})).await.unwrap();
    assert!(started.elapsed() < BACKGROUND_SETTLE + Duration::from_secs(2), "{:?}", started.elapsed());
    let text = typed.text_content();
    assert!(text.ends_with(&format!("[Running in the background as session {id}. Read what it prints with bash_output, or stop it with bash_input \"\\u0003\".]")), "{text}");
    t.host.get(&id).unwrap().stop("Stopped");

    // Answered, it prints on and on.
    let asking = call(&t.bash, json!({"command": format!("echo ready; read -r x; {ticks}")})).await.unwrap();
    let id = session_id(&asking);
    printed(&t.host.get(&id).unwrap(), "ready").await;
    let started = Instant::now();
    let typed = call(&t.input, json!({"session_id": id, "text": "go"})).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
    let text = typed.text_content();
    assert!(text.ends_with(&format!("[Still running as session {id}. Wait for more with bash_output, or answer it with bash_input.]")), "{text}");
    t.host.get(&id).unwrap().stop("Stopped");
}
```

Note: `longest_wait` must be reachable from tests — same module, so a plain private field works.
Existing `call()` helper has no timeout; the foreground half of the new test would hang forever on
the old code. Two options, both in-scope:

- minimal: wrap the two foreground `call(&t.input, …)` lines in
  `tokio::time::timeout(Duration::from_secs(30), …).await.expect("bash_input returned")`, or
- upstream-identical: also wrap `call()` itself with a 60 s timeout, as `fc3656d2` does.

Recommended: upstream-identical `call()` wrapper — it makes RED a fail instead of a hang, and it is
one helper.

## Hunk 3 — docs/architecture/terminal-sessions.md

Insert after the first clause of the "A call returns when the command exits…" paragraph (the one
ending `bash_input` and `bash_output` wait the same way, and `timeout` still kills the command`),
before `At a question the note quotes it.`:

> A command that keeps printing never goes quiet, so `bash_output` waits at most its `wait_seconds`
> (300 s at most, `MAX_OUTPUT_WAIT`) and `bash_input` at most 2 s for a command in the background
> (`BACKGROUND_SETTLE`) and 300 s for any other, then returns what it printed with a note that it
> still runs; a Ctrl-C the command takes late, or not at all, reads the same.

Local file has **no** `MAX_OUTPUT_WAIT` mention yet, so this sentence is new text, not an edit.

## Explicitly out of scope

- `crates/cli/src/turns.rs` test-harness 60 s timeout (different crate, no relation to the guard).
- `.github/workflows/test.yml` job `timeout-minutes` (CI, repository-level).
- The upstream edits to `a_prompt_on_dev_tty_returns_at_once_and_its_answer_stays_secret` and
  `a_background_command_returns_while_it_prints`: those match upstream-parent text; the local file
  has diverged, so applying them is a separate reconciliation, not this handoff.
- Cancellation, `stop`, `send_to_background`, `session_result` behavior.

## RED / GREEN commands

- RED (old code, after the test lands):
  `cargo test -p beans-agent tools::bash_session::tests::input_returns_while_the_command_keeps_printing -- --exact`
  → without the `call()` timeout wrapper the test hangs; with it, panic
  `{name} did not return within a minute`.
- GREEN: same command → `1 passed`.
- Guard regression sweep: `cargo test -p beans-agent tools::bash_session` (all module tests; expects
  the pre-existing count to hold, no new failures).
- Docs: `bun run check:docs`.