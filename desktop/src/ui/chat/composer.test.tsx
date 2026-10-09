// Run through the existing Solid Vite seam in a browser, without starting the app or CLI.
import { createSignal, flush, Show } from "solid-js";
import { render } from "@solidjs/web";
import { files, hostInfo } from "../../host";
import { setLanguage } from "../../l10n";
import { store } from "../../model/store";
import { ChatView } from "./ChatView";
import { PageMenuHost } from "../menu";

export async function checkComposerSendAck(): Promise<string[]> {
  const assert = (ok: unknown, message: string) => { if (!ok) throw new Error(message); };
  setLanguage("en");
  const internals = store as unknown as { transport: unknown; handle(name: string, data: unknown): void; emit(event: { kind: "identityChanged" | "connectionChanged" }): void; bootstrapGeneration: number; runningJobs: { id: string; chatID: string; botID: string }[] };
  const previous = internals.transport;
  const saved = { chats: store.chats, identityID: store.identityID, generation: internals.bootstrapGeneration, jobs: internals.runningJobs, isMock: hostInfo().isMock, choose: files.choose };
  const accountA = `sendack-account-A-${crypto.randomUUID()}`;
  const accountB = `sendack-account-B-${crypto.randomUUID()}`;
  store.identityID = accountA;
  hostInfo().isMock = false;
  files.choose = async () => [{ path: "/synthetic/sendack.txt", url: "", name: "sendack.txt", mime: "text/plain", size: 4, isFile: true }];
  const calls: { params: Record<string, unknown>; resolve(value: unknown): void; reject(error: Error): void }[] = [];
  internals.transport = { request: (method: string, params: Record<string, unknown>) => {
    if (method !== "chats.send") throw new Error(`Unexpected transport request: ${method}`);
    return new Promise((resolve, reject) => calls.push({ params, resolve, reject }));
  } };
  store.chats = [{ id: "sendack-chat", kind: "dm", botIDs: [], isPinned: false, createdAt: 1, messages: [
    { id: "quoted", author: { kind: "you" }, body: { kind: "text", text: "reply source" }, state: { kind: "complete" }, createdAt: 1, attachments: [] },
  ] }];
  store.chats.push({ id: "sendack-other", kind: "dm", botIDs: [], isPinned: false, createdAt: 1, messages: [] });
  const [chatID, setChatID] = createSignal("sendack-chat");
  const [mounted, setMounted] = createSignal(true);
  const redirects: string[] = [];
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><Show when={mounted()}><ChatView chatID={chatID()} onRedirect={(id) => redirects.push(id)} /></Show><PageMenuHost /></>, root);
  const settle = async () => { await new Promise((resolve) => setTimeout(resolve, 0)); flush(); };
  const field = () => root.querySelector<HTMLTextAreaElement>("textarea")!;
  const type = (value: string) => { field().value = value; field().dispatchEvent(new Event("input", { bubbles: true })); flush(); };
  const send = () => field().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  const acknowledge = (index: number) => calls[index].resolve({ message: {
    id: calls[index].params.message_id, chat_id: "sendack-chat", author: { kind: "you" },
    body: { kind: "text", text: calls[index].params.text }, state: { kind: "complete" }, created_at: 1,
  } });
  try {
    await settle();
    root.querySelector<HTMLElement>('[data-bubble="quoted"]')!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
    await settle();
    [...document.querySelectorAll<HTMLButtonElement>(".page-menu-item")].find((button) => button.textContent === "Reply")!.click();
    root.querySelector<HTMLButtonElement>('[aria-label="Attach files"]')!.click();
    await settle();
    assert(root.querySelector(".composer-reply") && root.querySelector(".composer-chip"), "Reply and file must enter actual draft");
    type("retained draft"); send(); await settle();
    assert(field().value === "retained draft", "Pending send must retain submitted text");
    send(); await settle(); assert(calls.length === 1, "Pending Enter must refuse double-send");
    calls[0].reject(new Error("Explicit API rejection")); await settle();
    assert(field().value === "retained draft", "Rejection must retain text");
    assert(root.querySelector(".composer-reply")?.textContent?.includes("reply source") && root.querySelector(".composer-chip")?.textContent?.includes("sendack.txt"), "Rejection must retain reply and attachment");
    assert(calls[0].params.reply_to === "quoted" && (calls[0].params.attachments as { name: string }[])[0].name === "sendack.txt", "Actual store must submit reply and attachment");
    assert(store.chat("sendack-chat")!.messages.every((message) => message.id !== calls[0].params.message_id), "Refusal must remove orphan optimistic row");
    assert(root.querySelector('[role="status"]')?.textContent?.includes("not confirmed"), "Failure must expose unconfirmed outcome");
    send(); await settle(); acknowledge(1); await settle();
    assert(field().value === "", "Acknowledgement must clear unchanged submitted revision");
    assert(!root.querySelector(".composer-reply") && !root.querySelector(".composer-chip"), "Acknowledgement must clear unchanged files and reply");
    type("submitted revision"); send(); await settle(); type("newer revision"); acknowledge(2); await settle();
    assert(field().value === "newer revision", "Acknowledgement must preserve newer edit");
    send(); await settle(); calls[3].resolve(null); await settle();
    assert(field().value === "newer revision", "Ambiguous result must retain draft");
    assert(calls.length === 4, "Ambiguous result must not retry automatically");
    send(); await settle();
    internals.handle("message.added", { chat_id: "sendack-chat", message: {
      id: calls[4].params.message_id, chat_id: "sendack-chat", author: { kind: "you" },
      body: { kind: "text", text: "newer revision" }, state: { kind: "complete" }, created_at: 1,
    } });
    calls[4].reject(new Error("Connection dropped after message event")); await settle();
    assert(store.chat("sendack-chat")!.messages.some((message) => message.id === calls[4].params.message_id), "Lost response must preserve event-confirmed row");
    assert(field().value === "newer revision" && calls.length === 5, "Lost response must retain draft without automatic retry");
    type("A submitted"); send(); await settle();
    setChatID("sendack-other"); await settle(); type("B edit"); acknowledge(5); await settle();
    assert(chatID() === "sendack-other" && redirects.length === 0 && field().value === "B edit", "Stale A acknowledgement must not redirect or clear B");
    setChatID("sendack-chat"); await settle(); assert(field().value === "", "Acknowledged A must not revive on chat return");
    type("Settings pending"); send(); await settle(); setMounted(false); await settle(); setMounted(true); await settle();
    send(); await settle(); assert(calls.length === 7 && field().value === "Settings pending", "Remount must retain pending ownership and refuse duplicate");
    type("Settings newer edit"); acknowledge(6); await settle();
    assert(field().value === "Settings newer edit", "Remounted newer edit must survive acknowledgement");
    send(); await settle(); setMounted(false); await settle(); acknowledge(7); await settle(); setMounted(true); await settle();
    assert(field().value === "", "Acknowledgement while in Settings must not revive submitted draft");
    store.chats = store.chats.map((chat) => ({ ...chat, botIDs: ["synthetic-bot"] }));
    type("old account pending"); send(); await settle();
    store.identityID = accountB; internals.bootstrapGeneration++;
    internals.runningJobs = []; internals.emit({ kind: "identityChanged" }); await settle();
    assert(field().value === "", "Reused chat ID must not expose other account draft");
    type("new account draft"); send(); await settle();
    const newPending = internals.runningJobs[0];
    calls[8].reject(new Error("Late old account failure")); await settle();
    assert(internals.runningJobs.includes(newPending) && field().value === "new account draft", "Late failure must not clear new account job or draft");
    acknowledge(9); await settle(); assert(field().value === "", "New account acknowledgement must clear only its draft");
    store.identityID = accountA; internals.emit({ kind: "identityChanged" }); await settle();
    assert(field().value === "old account pending" && root.querySelector('[role="status"]')?.textContent?.includes("not confirmed"), "Old account unsent intent and uncertainty must remain recoverable");
    type("generation pending"); send(); await settle();
    internals.bootstrapGeneration++; internals.emit({ kind: "connectionChanged" });
    internals.runningJobs = []; const replacement = store.send("replacement generation", [], [], "sendack-chat");
    await settle(); const replacementJob = internals.runningJobs[0];
    calls[10].reject(new Error("Late old generation failure")); await settle();
    assert(internals.runningJobs.includes(replacementJob) && field().value === "generation pending", "Late generation failure must preserve replacement job and intent");
    acknowledge(11); await replacement; await settle();
    type("account ABA submission"); send(); await settle();
    store.identityID = accountB; internals.emit({ kind: "identityChanged" }); await settle(); type("other account unsent");
    store.identityID = accountA; internals.emit({ kind: "identityChanged" }); await settle();
    acknowledge(12); await settle();
    assert(field().value === "account ABA submission", "Account ABA must invalidate old acknowledgement without losing intent");
    store.identityID = accountB; internals.emit({ kind: "identityChanged" }); await settle();
    assert(field().value === "other account unsent", "Old acknowledgement must not clear other account draft");
    return ["rejection/success/new-edit retention", "chat-switch stale acknowledgement", "Settings remount pending refusal", "Settings newer-edit retention", "Settings acknowledgement without revival", "account reused-ID isolation and recovery", "late account failure job ownership", "late generation failure job ownership"];
  } finally { dispose(); root.remove(); internals.transport = previous; store.chats = saved.chats; store.identityID = saved.identityID; internals.bootstrapGeneration = saved.generation; internals.runningJobs = saved.jobs; hostInfo().isMock = saved.isMock; files.choose = saved.choose; }
}
