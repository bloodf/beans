import { expect, test } from "bun:test";
import { routineProblem, routineProblemNeedsUser, routineScheduleSummary } from "./models";
import { toRoutine, type WireRoutine } from "./wire";

// The provider streak survives quiet checks; only the CLI's state decides current failure.
test("routine recovery keeps model and check failures separate and follows authoritative state", () => {
  const wire: WireRoutine = {
    id: "routine", bot_id: "bot", name: "Inbox", prompt: "Summarize new mail",
    schedule: "every 30m", schedule_text: "Every 30 minutes", timezone: "Pacific/Kiritimati",
    missed_run_policy: "skip", is_enabled: false, paused_reason: "authentication", state: "blocked",
    check: "return null;", created_at: 1, last_scheduled_at: 100,
    health: {
      updated_at: 100, last_check_at: 100, last_success_at: 100, status: "quiet",
      connection_failures: 0, authentication_failures: 0, retry_at: null,
      model: { status: "blocked", connection_failures: 0, authentication_failures: 3, retry_at: 400 },
    },
  };
  const routine = toRoutine(wire);
  expect(routineProblem(routine)).toBe("modelSignedOut");
  expect(routineProblemNeedsUser(routineProblem(routine))).toBe(true);
  expect(routine.health.lastCheckAt).toBe(100_000);
  expect(routine.health.model.retryAt).toBe(400_000);
  expect(routine.lastScheduledAt).toBe(100_000);
  expect(routineScheduleSummary(routine)).toBe(routine.scheduleText);
  expect(routineProblem({ ...routine, isRunning: true })).toBeUndefined();
  expect(routineProblem({ ...routine, state: "paused", pausedReason: undefined })).toBeUndefined();
  expect(routineProblem({ ...routine, state: "waiting_for_runner" })).toBe("offline");
  expect(routineProblem({ ...routine, state: "failed" })).toBe("modelSignInFailed");
  const checkFailure = toRoutine({ ...wire, state: "failed", paused_reason: null, is_enabled: true,
    health: { ...wire.health!, status: "failed", connection_failures: 2,
      model: { status: null, connection_failures: 0, authentication_failures: 0, retry_at: null } } });
  expect(routineProblem(checkFailure)).toBe("cantConnect");
  expect(routineProblemNeedsUser(routineProblem(checkFailure))).toBe(false);
  for (const status of ["quiet", "ready"]) {
    expect(routineProblem({ ...checkFailure, health: { ...checkFailure.health, model: { ...checkFailure.health.model, status } } })).toBe("cantConnect");
  }
  expect(routineProblem({ ...checkFailure, state: "blocked" })).toBe("checkBlocked");
});
