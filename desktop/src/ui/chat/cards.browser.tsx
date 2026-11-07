// Browser consumer check: serve through the desktop Solid Vite plugin and call
// checkCommandCardInput() from an installed browser runner. No CLI or account is used.
import { createSignal, flush } from "solid-js";
import { render } from "@solidjs/web";
import { setLanguage } from "../../l10n";
import type { CommandRun } from "../../model/models";
import { CommandCard } from "./cards";

export async function checkCommandCardInput(): Promise<void> {
  const assert = (condition: unknown, message: string) => {
    if (!condition) throw new Error(message);
  };
  setLanguage("en");
  const root = document.createElement("div");
  document.body.append(root);
  const initial: CommandRun = {
    command: "read -r NAME </dev/tty", state: "running", sessionID: "consumer-session",
    handedOver: true, background: false,
  };
  const [run, setRun] = createSignal(initial);
  const sent: string[] = [];
  let complete: (() => void) | undefined;
  let fail: ((error: Error) => void) | undefined;
  const dispose = render(() => <CommandCard
    run={run()} messageID="consumer-message" botName="Chef" groupStart={true}
    onDecision={() => { throw new Error("Unexpected permission decision"); }}
    onShowCommand={() => {}} onStop={async () => {}}
    onSend={(text) => {
      sent.push(text);
      return new Promise<void>((resolve, reject) => { complete = resolve; fail = reject; });
    }}
  />, root);
  const settle = async () => { await Promise.resolve(); flush(); };
  const field = () => root.querySelector("input")!;
  const sendButton = () => [...root.querySelectorAll("button")].find((button) => button.textContent === "Send input")!;
  try {
    await settle();
    assert(root.querySelector(".card-title")?.textContent === "Chef's command is running", "Blind input must keep Running title");
    assert(field()?.type === "password", "Running session must offer secure input without a prompt");
    assert(sendButton(), "Running session must offer explicit Send input");
    field().value = "private-consumer-sentinel";
    field().dispatchEvent(new Event("input", { bubbles: true }));
    flush();
    sendButton().click();
    await settle();
    assert(sent[0] === "private-consumer-sentinel", "Input callback must receive the draft");
    assert(field().disabled && sendButton().disabled, "Pending input must disable controls");
    field().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    assert(sent.length === 1, "Pending submission must not repeat");
    fail!(new Error("transport private-consumer-sentinel"));
    await settle();
    assert(field().value === "private-consumer-sentinel", "Failure must retain draft");
    assert(root.querySelector(".card-note.error")?.textContent === "Could not send input to the command", "Failure must use fixed error");
    assert(!root.textContent?.includes("private-consumer-sentinel") && !root.innerHTML.includes("transport private-consumer-sentinel"), "Error must not reflect input or transport details");
    field().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();
    assert(sent.length === 2, "Enter must submit retained draft");
    complete!();
    await settle();
    assert(field().value === "", "Success must clear draft");
    for (const next of [{ ...initial, state: "exited" as const }, { ...initial, sessionID: undefined }, { ...initial, handedOver: false }]) {
      const oldButton = sendButton();
      const oldField = field();
      setRun(next);
      await settle();
      assert(!root.querySelector("input") && !sendButton(), "Unavailable cards must hide input");
      oldButton.click();
      oldField.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await settle();
      assert(sent.length === 2, "Stale controls must refuse unavailable input");
      setRun(initial);
      await settle();
    }
  } finally {
    dispose();
    root.remove();
  }
}
