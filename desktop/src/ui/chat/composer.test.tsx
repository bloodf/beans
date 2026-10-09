// Run through the existing Solid Vite seam in a browser, without starting the app or CLI.
import { flush } from "solid-js";
import { render } from "@solidjs/web";
import { files, hostInfo } from "../../host";
import { setLanguage } from "../../l10n";
import { store } from "../../model/store";
import { ChatView } from "./ChatView";
import { PageMenuHost } from "../menu";

export async function checkComposerSendAck(): Promise<string[]> {
  const assert = (ok: unknown, message: string) => { if (!ok) throw new Error(message); };
  setLanguage("en");
  const internals = store as unknown as { transport: unknown; handle(name: string, data: unknown): void };
  const previous = internals.transport;
  const saved = { chats: store.chats, isMock: hostInfo().isMock, choose: files.choose };
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
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><ChatView chatID="sendack-chat" onRedirect={() => { throw new Error("Unexpected redirect"); }} /><PageMenuHost /></>, root);
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
    return ["pending retention", "double-send refusal", "API rejection text/file/reply retention", "optimistic refusal cleanup", "acknowledged text/file/reply clear", "edit while pending", "ambiguous retention/no retry", "event-before-lost-response preservation"];
  } finally { dispose(); root.remove(); internals.transport = previous; store.chats = saved.chats; hostInfo().isMock = saved.isMock; files.choose = saved.choose; }
}
