# Android #252 cancel fixture — test-only handoff (2026-10-12)

Status: **corrected exact hunk frozen below. Not applied. No source edit, no Gradle, no SDK, no
network resolve.** Original Android #252 source owner handoff is still required before any write;
the branch stays HOLD.

## Bound head

- Branch: `roadmap/p0/issue-252-android-updates`
- Full SHA: `0cc7c294e7a2fc05ad1e957e347463c4fc310e4d`
- Test file: `mobile/modules/beans-core/android/src/test/java/app/beans/core/UpdateOperationTest.kt`
- Production file (NOT to be touched): `mobile/modules/beans-core/android/src/main/java/app/beans/core/UpdateOperation.kt`

These Android paths exist only on that branch. `7a0ba62` is not an ancestor of `main`; `main` has no
`UpdateOperation` sources. Bind any edit to the SHA above.

## The false positive (why the first one-line proposal was not enough)

Fixture sends `Content-Length: 2` headers and never a body, so the client blocks in `input.read`.
The server then closed the socket by itself after 3 s, unblocking that read with no cancel involved.
The assertion only proved *some* exception fired — the submitted lambda returns true for any,
including `IllegalStateException` from `ensureActive()`.

Second, larger hole: `operation.cancel()` ran **on the test thread and untimed**. `cancel()` calls
`connection?.disconnect()` synchronously. If that call ever blocked, the test would hang before
`request.get(2, TimeUnit.SECONDS)` could time anything out — the timeout would never protect the run.

Third: `ensureActive()` only proves the new object is constructed. It never proves a fresh operation
completes a real request after the cancel.

## Frozen test-only hunk

Replace **only** the method `cancellationDisconnectsBlockedMetadataAndAllowsFreshOperation` in
`UpdateOperationTest.kt` with the block below. Nothing else in the file, nothing in
`UpdateOperation.kt`, no new fixture class, no new test name.

```kotlin
  @Test fun cancellationDisconnectsBlockedMetadataAndAllowsFreshOperation() {
    ServerSocket(0).use { server ->
      val received = CountDownLatch(1)
      val release = CountDownLatch(1)
      val pool = Executors.newFixedThreadPool(3)
      val open = java.util.Collections.newSetFromMap(java.util.concurrent.ConcurrentHashMap<java.net.Socket, Boolean>())
      var failure: Throwable? = null
      try {
        // The first request gets headers only: the client blocks in the body read and nothing but
        // cancel can end it, because this server never closes on its own.
        // The second request is answered in full, for a fresh operation after the cancel.
        val serving = pool.submit {
          server.accept().use { blocked ->
            open.add(blocked)
            val reader = blocked.getInputStream().bufferedReader()
            while (!reader.readLine().isNullOrEmpty()) { }
            blocked.getOutputStream().write("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n".toByteArray())
            blocked.getOutputStream().flush()
            received.countDown()
            release.await()
          }
          server.accept().use { fresh ->
            open.add(fresh)
            val reader = fresh.getInputStream().bufferedReader()
            while (!reader.readLine().isNullOrEmpty()) { }
            fresh.getOutputStream().write("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".toByteArray())
            fresh.getOutputStream().flush()
          }
        }
        UpdateOperation(5000) { URI("http://127.0.0.1:${server.localPort}/").toURL().openConnection() as HttpURLConnection }.use { operation ->
          val output = ByteArrayOutputStream()
          val request = pool.submit<Boolean> { try { operation.fetch(url, 100, output); false } catch (_: Exception) { true } }
          assertTrue(received.await(2, TimeUnit.SECONDS))

          // cancel() is timed inside the callable, so the measured span is the call itself and not
          // the wait for the future: a disconnect that hangs fails the test instead of stalling it,
          // and the blocked read can only end after this returns.
          val cancelled = pool.submit<Long> {
            val before = System.nanoTime()
            operation.cancel()
            System.nanoTime() - before
          }
          val took = cancelled.get(2, TimeUnit.SECONDS)
          assertTrue(took < TimeUnit.SECONDS.toNanos(1), "cancel() took ${TimeUnit.NANOSECONDS.toMillis(took)}ms")
          assertTrue(request.get(2, TimeUnit.SECONDS), "the blocked read did not end at cancel")
          assertEquals(0, output.size())
        }
        release.countDown()
        serving.get(3, TimeUnit.SECONDS)
        // A fresh operation completes a real request, not just ensureActive().
        UpdateOperation(5000) { URI("http://127.0.0.1:${server.localPort}/").toURL().openConnection() as HttpURLConnection }.use { fresh ->
          val out = ByteArrayOutputStream()
          assertEquals(2L, fresh.fetch(url, 100, out))
          assertEquals("ok", out.toString())
        }
      } catch (t: Throwable) {
        failure = t
        throw t
      } finally {
        // Cleanup never swallows a failure of its own and never masks the assertion that failed
        // first: every problem found here rides along on whichever throwable the run ends with.
        val problems = mutableListOf<Throwable>()
        release.countDown()
        // The listener closes first, before any socket is swept, so no accept can start after the
        // sweep and leave a socket behind that the set never saw. The outer `use` closes it again
        // on the way out, which is a no-op on an already closed server socket.
        try { server.close() } catch (t: Throwable) { problems.add(t) }
        pool.shutdownNow()
        for (socket in open) {
          try { socket.close() } catch (t: Throwable) { problems.add(t) }
        }
        // An interrupt here must not skip the checks below or replace the primary throwable, so it
        // is caught, recorded, and the flag is restored only once every check has run.
        var interrupted = false
        try {
          if (!pool.awaitTermination(5, TimeUnit.SECONDS)) {
            problems.add(IllegalStateException("fixture threads did not stop"))
          }
        } catch (t: InterruptedException) {
          interrupted = true
          problems.add(t)
        } catch (t: Throwable) {
          problems.add(t)
        }
        // Swept after the pool has ended, so a socket accepted during shutdown is seen here too.
        for (socket in open) {
          val closed = try { socket.isClosed } catch (t: Throwable) { problems.add(t); true }
          if (!closed) problems.add(IllegalStateException("a fixture socket is still open"))
        }
        if (interrupted) Thread.currentThread().interrupt()
        val pending = failure
        if (pending == null) {
          problems.firstOrNull()?.let { first ->
            for (extra in problems.drop(1)) first.addSuppressed(extra)
            throw first
          }
        } else {
          for (problem in problems) pending.addSuppressed(problem)
        }
      }
    }
  }
```


## Why each piece is required

- **No self-close.** `release.await()` with no timeout is the whole point: the server thread cannot
  rescue the blocked read. The `finally` block still frees it on every exit path, including a failed
  assert, so no thread is left parked.
- **cancel() timed inside the callable.** The callable reads `System.nanoTime()` before and after
  `operation.cancel()` and returns the elapsed span, so the assertion measures the call itself and
  not the time the future sat in the executor queue. `get(2, TimeUnit.SECONDS)` on top of that fails
  with `TimeoutException` if `disconnect()` blocks at all. The test thread never calls `cancel()`
  directly.
- **The blocked read is proven to end at cancel.** `request.get(2, TimeUnit.SECONDS)` can now only be
  satisfied by the disconnect, since nothing else closes that socket, and cancel() has already
  returned before it is called.
- **Real fresh request.** The second accept serves a full `Content-Length: 2` body; a brand new
  `UpdateOperation` must fetch it and get `2` bytes / `ok`, which `ensureActive()` alone never showed.
- **Cleanup failures are surfaced, never swallowed.** `runCatching` is gone. `finally` counts the
  latch down, calls `shutdownNow()`, closes each accepted socket, and collects *every* problem it
  meets — a `close()` that throws, a pool that will not terminate inside 5 s, a socket still open —
  into `problems`. Nothing is discarded.
- **An interrupt cannot cut cleanup short.** `pool.awaitTermination` throws `InterruptedException` when
  the thread is interrupted, which would have replaced the primary throwable and skipped the
  `isClosed` sweep. It is now caught separately, recorded in `problems`, and the flag is restored
  with `Thread.currentThread().interrupt()` only after every remaining check has run and before
  anything is thrown, so the interrupt is preserved rather than lost or leaked into the sweep.
- **The listener closes before the socket sweep.** `ServerSocket.use` only closes the listener after
  `finally` has already run, so the serving thread's second `server.accept()` could accept a socket
  during cleanup and add it to `open` after the sweep had passed. `finally` now calls `server.close()`
  first — before `shutdownNow()` and before any socket is closed — so no accept can begin after the
  set is swept; the outer `use` close is then a no-op. The `isClosed` pass runs again *after* the pool
  has terminated, so a socket accepted during shutdown is still caught.
- **Cleanup cannot mask the first failure, or be skipped by it.** The body is wrapped in
  `catch (t: Throwable) { failure = t; throw t }`, so `finally` always runs. A cleanup problem with
  no primary failure is thrown, with the rest attached as `suppressed`; with a primary failure
  already propagating, each cleanup problem is `addSuppressed`-ed onto it instead of replacing it.
  The every-socket-closed check runs *inside* `finally`, so an assertion failure earlier in the test
  can no longer skip it.

## Production behavior unchanged

`UpdateOperation.kt` is not touched. `cancel()` still sets `stopped`, clears `active`, and
disconnects; `fetch` still calls `ensureActive()` around each read and clears `active` in `finally`.
This hunk only makes the existing behavior observable.

## Residual cleanup race — NOT closed, do not read the cleanup as proved

`server.close()` stops *further* accepts. It does not un-register a socket the serving thread has
already accepted. Two sequences remain open:

1. The serving thread is inside the second `server.accept()` and has returned a socket when the
   listener closes; it then executes `open.add(fresh)` **after** both cleanup passes have finished
   iterating. That socket is never closed and never inspected.
2. The same add lands between the close-sweep and the post-termination `isClosed` pass: the second
   pass sees the socket but only *reads* `isClosed`, it does not close what it finds.

Iteration over a `ConcurrentHashMap`-backed set is weakly consistent, so neither pass is a snapshot
guarantee. The hunk therefore bounds the socket set in every ordinary path but **does not prove**
every accepted socket is closed. An actual applier or reviewer should close late registrations inside
the serving thread's own `use` block (which already runs) and treat that thread's termination as the
real ordering guarantee, rather than relying on the cleanup sweep.

This is recorded so nobody reads "every fixture socket is closed" as a proved property of the frozen
hunk. It is a residual cleanup race in a test fixture, not a production defect:
`UpdateOperation.kt` is untouched.

## Runnable check — still NOT available, zero PASS claimed

- Intended: `./gradlew :beans-core:testDebugUnitTest --tests app.beans.core.UpdateOperationTest` from
  `mobile/modules/beans-core/android` (junit `4.13.2` per that `build.gradle`).
- Not executed here: no Gradle, no Android SDK build, no dependency install, no network resolve.
  `~/.gradle/caches/modules-2/files-2.1/` has no cached `junit` or `hamcrest`.
- Whoever applies the hunk must record the real runner output and pass count. This document asserts
  no result.

## Ownership

Original Android #252 source owner lease is not held here; not released, not assumed. This file is a
frozen proposal against `0cc7c294e7a2fc05ad1e957e347463c4fc310e4d` only.

## Owner / disposition packet (read-only trace, no ghost restart)

Traced from retained receipts only. No ledger was written, no corrupt helper imported, no test
edited, no worker restarted.

**Lineage is a single commit.** `git log -- mobile/modules/beans-core/android/src/test/java/app/beans/core/UpdateOperationTest.kt`
on `roadmap/p0/issue-252-android-updates` returns exactly one commit:
`7a0ba62e` "fix(mobile): cancel update transport and recover composer drafts", Heitor Ramon Ribeiro,
2026-10-08 13:36:51 -0300. No later commit on that branch touches the file, so `0cc7c294` carries the
original fixture unchanged.

**Authoring session receipt exists.** `sessions/fix-android-review/2026-10-08T16-30-31-760Z_01a11c5a-2350-70d3-ad6d-ddebf30cffe8.jsonl`
line 89 is the assistant `write` tool call that created `UpdateOperationTest.kt` at 16:33:55Z ("Adding
transport cancellation regressions"). The same session later deleted its own throwaway
`/tmp/beans-252-owner-check.ts` Gradle-cache probe.

**The review that flagged it is retained, and it is the origin of these findings.**
`evidence/verify-android-fixes.md` resolves `7a0ba62^{commit}` to
`7a0ba62edf5b0f620c680e15d1b94baed7ab91d0` and reads sources by `git show <fullSHA>:<path>`. Verdict
line 7: "Changes requested: F2 is not fully closed... No overall F1/F2 approval." Lines 21-28 name
precisely the two fixture flaws this hunk fixes: the server's independent `release.await(3, ...)`
close, the untimed synchronous `cancel()`, and the fresh-operation check that only calls
`ensureActive()`. That document also states no Kotlin test was run and `kotlinc` was absent.

**Historical assignment, unretired.** `capacity-issue-ownership.json` records issue 252 as worker
"beans-android-252 remote lead allocated", pane `w7:p3`, state "Occupied execution slot, current
working/idle census must be observed, not perpetual claim", gate: actual minimum Kotlin/Expo/UniFFI
bridge compile is a genuine merge blocker. That names an execution slot, **not** a current authority,
and no receipt binds it to a session id or process.

**Disposition: unchanged and still blocked.** `evidence/retained-owner-manual-batch-20261012.json`
task `RETAINED-P0-252-CANCEL-FIXTURE-HANDOFF` is `BLOCKED_NEW_SOURCE_BINDING`,
`actionready: false`, owner `phone280-finite-guard-20261012` for the *proposal only*, reason: "Actual
peer owns proposal, not original #252 source lease." `capacity-decision-dispositions.json` shows
#252 delivery "Draft PR255 frozen 0cc7c294 tiny-diff approved after 659ab63 host evidence", remaining
"Actual exact generated Kotlin/Expo compile/device/installer/UI; lane2 unavailable". Neither
`0cc7c294` nor `8ad4553` is an ancestor of the Head-selected base, and no receipt shows an approving
independent review for #252 as a whole.

**Residual owner uncertainty, preserved.** (1) Whether the `w7:p3` remote lead is the same worker
that authored `7a0ba62e`, or a distinct later allocation, cannot be settled from the retained
receipts. (2) No receipt retires or transfers that lead's lease, so it is unresolved, not released.
(3) The retained batch names my historical session id as a "historical receipt; current exact
process/session needs Main binding" — that binding is Main's to make, not mine to assert. Only Main
can bind the exact branch/hunk ownership that unblocks the write.