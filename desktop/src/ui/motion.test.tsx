// Browser consumer check: serve through the desktop Solid Vite plugin and call
// checkReducedMotion() from an installed browser runner. No CLI or account is used.
import { flush } from "solid-js";
import { render } from "@solidjs/web";
import { setLanguage } from "../l10n";
import { presentSheet, Sheet, SheetHost } from "./overlay";
import { Spinner } from "./controls";
import { WorkingCell } from "./chat/cells";
import type { Bot } from "../model/models";
import "./components.css";
import "./layout.css";

/** What the browser reports for one element's animation, the way a reader of the page sees it. */
function animating(selector: string): number {
  const element = document.querySelector(selector);
  if (!element) throw new Error(`Missing element ${selector}`);
  return element.getAnimations().length;
}

export async function checkReducedMotion(): Promise<Record<string, string>> {
  const assert = (condition: unknown, message: string) => {
    if (!condition) throw new Error(message);
  };
  setLanguage("en");
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <div><SheetHost /><Spinner /><WorkingCell bots={[{ id: "bot-1", name: "Chef" } as Bot]} chatID="chat-1" activity="Reading a file" showsName={true} /></div>, root);
  const settle = async () => { await Promise.resolve(); flush(); await new Promise((r) => setTimeout(r, 0)); flush(); };
  try {
    const sheet = presentSheet((dismiss) => <Sheet title="Motion check" onCancel={dismiss} />);
    await settle();
    const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
    assert(reduce || !reduce, "The media query must answer");
    const result = {
      scrim: animating(".sheet-scrim"),
      frame: animating(".sheet-frame"),
      spinner: animating(".spinner"),
      working: animating(".working-text"),
    };
    assert(root.querySelector(".working-text")?.textContent?.includes("Reading a file"), "The working row must keep its activity text when still");
    sheet.dismiss();
    await settle();
    if (reduce) {
      assert(result.scrim === 0, `The scrim must not animate under Reduce Motion, saw ${result.scrim}`);
      assert(result.frame === 0, `The sheet frame must not animate under Reduce Motion, saw ${result.frame}`);
      assert(result.spinner === 0, `The spinner must not animate under Reduce Motion, saw ${result.spinner}`);
      assert(result.working === 0, `The working row must not shimmer under Reduce Motion, saw ${result.working}`);
      return { setting: "reduce", scrim: "still", frame: "still", spinner: "still", working: "still" };
    }
    assert(result.scrim > 0, "The scrim must animate when motion is allowed");
    assert(result.frame > 0, "The sheet frame must animate when motion is allowed");
    assert(result.spinner > 0, "The spinner must animate when motion is allowed");
    assert(result.working > 0, "The working row must shimmer when motion is allowed");
    return { setting: "no-preference", scrim: "moving", frame: "moving", spinner: "moving", working: "moving" };
  } finally {
    dispose();
    root.remove();
  }
}