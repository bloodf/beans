// Routine details follow the CLI's state, persisted health, timezone and missed-run policy.
// The bot owns edits through chat; actions use the existing routine requests.

import { createEffect, createMemo, Show } from "solid-js";
import { L } from "../../l10n";
import * as Format from "../../model/format";
import { lastCheckSummary, lastRunSummary, routineProblem, routineProblemExplanation, routineProblemNeedsUser, routineProblemText, routineScheduleSummary, type Bot } from "../../model/models";
import { track } from "../../model/reactive";
import { store } from "../../model/store";
import { Button } from "../controls";
import { alert, presentSheet, Sheet } from "../overlay";
import { KeyValueRow, NoteRow, Section } from "../sections";

/** `onEditInChat` puts text in the chat's composer, for the bot to change the routine. */
export function presentRoutine(routineID: string, bot: Bot, onEditInChat: (text: string) => void): void {
  presentSheet((dismiss) => <RoutineSheet routineID={routineID} bot={bot} onEditInChat={onEditInChat} dismiss={dismiss} />);
}

function RoutineSheet(props: { routineID: string; bot: Bot; onEditInChat: (text: string) => void; dismiss: () => void }) {
  const title = store.routine(props.routineID)?.name ?? L("Routine");
  const routine = createMemo(() => {
    track.roster();
    track.chats();
    return store.routinesFor(props.bot.id).find((candidate) => candidate.id === props.routineID);
  });
  // The sheet closes when the routine is gone.
  createEffect(
    () => routine() === undefined,
    (gone) => {
      if (gone) props.dismiss();
    },
  );

  const state = (): [string, string] => {
    const current = routine();
    if (!current) return ["", "var(--label-2)"];
    if (current.isRunning || current.state === "running") return [L("Running…"), "var(--accent)"];
    const problem = routineProblem(current);
    if (problem) return [routineProblemText(problem), routineProblemNeedsUser(problem) ? "var(--orange)" : "var(--label-2)"];
    if (current.isEnabled) return [L("On"), "var(--green)"];
    return [current.pausedReason === "away" ? L("Paused while you were away") : L("Paused"), "var(--label-2)"];
  };
  const runTooltip = () => {
    track.roster();
    const runner = store.device(props.bot.runnerID);
    return runner ? L("Runs on %@ now", runner.name) : L("Runs on the bot's Runner now");
  };
  const runnerName = () => {
    track.roster();
    return store.device(props.bot.runnerID)?.name ?? L("its Runner");
  };

  const confirmDelete = async () => {
    const current = store.routine(props.routineID);
    if (!current) return;
    const answer = await alert({
      message: L("Delete “%@”?", current.name),
      informative: L("This deletes the routine and stops its future runs. This can't be undone."),
      style: "warning",
      buttons: [{ title: L("Delete routine") }, { title: L("Cancel") }],
    });
    if (answer !== 0) return;
    store.deleteRoutine(props.routineID);
    props.dismiss();
  };

  return (
    <Sheet
      title={title}
      subtitle={L("A task %@ runs on its own in this chat. %@ set it up and can change it: ask in chat.", props.bot.name, props.bot.name)}
      width={520}
      class="routine-sheet"
      confirm={L("Done")}
      cancel={null}
      onConfirm={props.dismiss}
      onCancel={props.dismiss}
      leading={
        <Show when={routine()}>
          {(current) => (
            <div class="sheet-actions">
              <Button disabled={current().isRunning || current().state === "running" || current().state === "waiting_for_runner" || current().pausedReason === "authentication"} tooltip={runTooltip()} onClick={() => store.runRoutine(props.routineID)}>
                {L("Run Now")}
              </Button>
              <Button onClick={() => store.setRoutineEnabled(props.routineID, !current().isEnabled)}>{current().isEnabled ? L("Pause") : L("Resume")}</Button>
              <Button onClick={() => {
                props.onEditInChat(L('Edit my routine "%@": ', current().name));
                props.dismiss();
              }}>{L("Edit in Chat…")}</Button>
              <Button onClick={() => void confirmDelete()}>{L("Delete…")}</Button>
            </div>
          )}
        </Show>
      }
    >
      <Show when={routine()}>
        {(current) => (
          <>
            <Section title={L("Schedule")}>
              <KeyValueRow label={L("State")} value={state()[0]} tint={state()[1]} />
              <Show when={routineProblem(current())}>
                {(problem) => <NoteRow text={routineProblemExplanation(problem(), props.bot.name, runnerName())} />}
              </Show>
              <KeyValueRow label={L("Schedule")} value={routineScheduleSummary(current())} tooltip={current().schedule.startsWith("every ") ? current().schedule : `${current().schedule} · ${current().timezone}`} />
              <KeyValueRow
                label={current().check === undefined ? L("Next run") : L("Next check")}
                value={current().nextRunAt !== undefined ? Format.upcoming(current().nextRunAt!) : "—"}
              />
              <Show when={current().isEnabled && Math.max(current().health.retryAt ?? 0, current().health.model.retryAt ?? 0)}>
                {(retryAt) => <KeyValueRow label={L("Retry no earlier than")} value={Format.daySeparator(retryAt())} />}
              </Show>
              <KeyValueRow
                label={L("Missed runs")}
                value={current().missedRunPolicy === "skip" ? L("Skip") : L("Run once")}
                tooltip={current().missedRunPolicy === "skip" ? L("When %@ was off at a scheduled time, the routine waits for the next one.", runnerName()) : L("When %@ was off at a scheduled time, the routine runs once when it’s back.", runnerName())}
              />
              <Button onClick={() => {
                props.onEditInChat(L('Edit the timezone and missed-run policy of my routine "%@". Current timezone: %@. Current missed_run_policy: %@ (coalesce runs once after missed times; skip waits for the next time). Change them to: ', current().name, current().timezone, current().missedRunPolicy));
                props.dismiss();
              }}>{L("Edit timezone and missed runs in Chat…")}</Button>
              <Show when={lastCheckSummary(current())}>
                {(summary) => <KeyValueRow label={L("Last check")} value={summary()} />}
              </Show>
              <Show when={current().check !== undefined && (current().health.status === "failed" || current().health.status === "blocked")}>
                <KeyValueRow label={L("Last successful check")} value={current().health.lastSuccessAt === undefined ? L("Never") : Format.daySeparator(current().health.lastSuccessAt!)} />
              </Show>
              <KeyValueRow label={L("Last run")} value={lastRunSummary(current())} />
            </Section>
            <Section title={L("Task")}>
              <div class="routine-prompt selectable">{current().prompt}</div>
            </Section>
            <Show when={current().check}>
              {(check) => (
                <Section title={L("Check")}>
                  <div class="routine-check selectable">{check()}</div>
                </Section>
              )}
            </Show>
          </>
        )}
      </Show>
    </Sheet>
  );
}
