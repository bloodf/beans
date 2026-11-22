import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { start, type Harness } from "./harness";

let harness: Harness;
beforeAll(async () => { harness = await start(); });
afterAll(async () => { await harness?.close(); });

const chats = (page: import("playwright").Page) => page.getByRole("listbox").first().getByRole("option");

describe("demo workspace", () => {
  test("loads the sidebar and the selected chat's transcript without page errors", async () => {
    const { page, errors } = await harness.open();
    expect(await chats(page).count()).toBeGreaterThanOrEqual(5);
    await expect.poll(() => page.getByRole("log", { name: "Transcript" }).innerText()).toContain("Launch review");
    expect(await page.title()).toContain("Beans");
    expect(errors).toEqual([]);
    await page.close();
  });

  test("sending in a bot's chat shows the message, clears the composer and gets the bot's reply", async () => {
    const { page, errors } = await harness.open();
    await chats(page).filter({ hasText: "Writer" }).first().click();
    const composer = page.locator("textarea.composer-input");
    await expect.poll(() => composer.getAttribute("aria-label")).toBe("Message Writer");
    const transcript = page.getByRole("log", { name: "Transcript" });
    const before = (await transcript.innerText()).length;
    await composer.fill("Please draft a haiku about beans");
    await composer.press("Enter");
    await transcript.getByText("Please draft a haiku about beans").waitFor();
    expect(await composer.inputValue()).toBe("");
    // The demo engine answers after a short delay; the reply lands under the sent message.
    await expect.poll(async () => {
      const text = await transcript.innerText();
      const after = text.slice(text.lastIndexOf("Please draft a haiku about beans"));
      return after.split("\n").filter((line) => line.trim()).length;
    }, { timeout: 20_000 }).toBeGreaterThan(2);
    expect((await transcript.innerText()).length).toBeGreaterThan(before);
    expect(errors).toEqual([]);
    await page.close();
  });

  test("Shift+Enter keeps writing instead of sending", async () => {
    const { page } = await harness.open();
    await chats(page).filter({ hasText: "Developer" }).first().click();
    const composer = page.locator("textarea.composer-input");
    await composer.fill("first line");
    await composer.press("Shift+Enter");
    await composer.type("second line");
    expect(await composer.inputValue()).toBe("first line\nsecond line");
    await page.close();
  });

  test("Search opens the palette, filters actions and runs the picked one", async () => {
    const { page, errors } = await harness.open();
    // The sidebar field is a read-only trigger; the palette owns the query. This checks actions
    // and Settings entries; the demo's content search has its own test below.
    await page.getByPlaceholder("Search").first().click();
    const dialog = page.getByRole("dialog").last();
    await expect.poll(() => dialog.innerText()).toContain("New Bot…");
    const query = dialog.locator("input").first();
    await query.fill("Group");
    await expect.poll(() => dialog.innerText()).toContain("New Group Chat…");
    expect(await dialog.innerText()).not.toContain("Pair a Device…");
    await query.fill("Writer");
    await expect.poll(() => dialog.innerText()).toContain("Writer");
    await query.fill("New Bot");
    await page.keyboard.press("Enter");
    // The action opens its sheet in place of the palette.
    await expect.poll(async () => (await page.getByRole("dialog").last().innerText()).includes("New Bot")).toBe(true);
    await page.keyboard.press("Escape");
    expect(errors).toEqual([]);
    await page.close();
  });

  test("the demo's search finds related messages and pictures by meaning (#586)", async () => {
    const { page, errors } = await harness.open();
    await page.getByPlaceholder("Search").first().click();
    const dialog = page.getByRole("dialog").last();
    await dialog.locator("input").first().fill("ship");
    await expect.poll(() => dialog.innerText()).toContain("Related");
    await expect.poll(() => dialog.innerText()).toContain("launch-banner.png");
    expect(await dialog.innerText()).toContain("Pictures");
    await page.keyboard.press("Escape");
    expect(errors).toEqual([]);
    await page.close();
  });

  test("Settings › Search shows the demo Runner's models instead of an error (#586)", async () => {
    const { page, errors } = await harness.open();
    await page.keyboard.press("Control+Comma");
    await page.getByText("Search", { exact: true }).last().click();
    await expect.poll(() => page.innerText("body")).toContain("1284 messages indexed");
    const body = await page.innerText("body");
    expect(body).toContain("37 pictures indexed");
    expect(body).toContain("SigLIP Base");
    expect(body).toContain("Local models use 740 MB on Workbench.");
    expect(body).not.toContain("not running");
    expect(errors).toEqual([]);
    await page.close();
  });

  test("Settings opens with its panes and returns to the chats", async () => {
    const { page, errors } = await harness.open();
    await page.keyboard.press("Control+Comma");
    for (const pane of ["General", "Providers", "Auto-review", "Plugins", "Bots", "Devices", "Advanced"]) {
      await page.getByText(pane, { exact: true }).first().waitFor();
    }
    await page.getByText("Devices", { exact: true }).first().click();
    await expect.poll(() => page.innerText("body")).toMatch(/Studio|Workbench|Runner/);
    await page.getByText("Back", { exact: true }).first().click();
    await chats(page).first().waitFor();
    expect(errors).toEqual([]);
    await page.close();
  });

  test("the command palette opens with Ctrl+K and closes with Escape", async () => {
    const { page } = await harness.open();
    await page.keyboard.press("Control+K");
    const dialog = page.getByRole("dialog").last();
    await dialog.waitFor();
    await page.keyboard.press("Escape");
    await expect.poll(() => page.getByRole("dialog").count()).toBe(0);
    await page.close();
  });

  test("App Language switches the interface to Simplified Chinese and back", async () => {
    const { page, errors } = await harness.open();
    await page.keyboard.press("Control+Comma");
    // A browser tab draws the system menu in the page.
    await page.getByRole("button", { name: "App Language" }).click();
    await page.locator(".page-menu-item", { hasText: "简体中文" }).click();
    await expect.poll(() => page.innerText("body"), { timeout: 10_000 }).toContain("通用");
    expect(await page.innerText("body")).toContain("服务商");
    await page.getByRole("button", { name: "应用语言" }).click();
    await page.locator(".page-menu-item", { hasText: "English" }).click();
    await expect.poll(() => page.innerText("body"), { timeout: 10_000 }).toContain("General");
    expect(errors).toEqual([]);
    await page.close();
  });

  test("New Bot creates a bot with its direct chat selected", async () => {
    const { page, errors } = await harness.open();
    await page.keyboard.press("Control+N");
    const sheet = page.getByRole("dialog").last();
    await sheet.getByPlaceholder("Name").fill("Tester");
    await sheet.getByPlaceholder("What it does and how it should work").fill("Checks every release.");
    await sheet.getByRole("button", { name: "Create Bot" }).click();
    await expect.poll(() => page.getByRole("dialog").count()).toBe(0);
    await expect.poll(() => chats(page).allInnerTexts()).toEqual(expect.arrayContaining([expect.stringContaining("Tester")]));
    expect(await page.locator("textarea.composer-input").getAttribute("aria-label")).toBe("Message Tester");
    expect(errors).toEqual([]);
    await page.close();
  });

  test("a chat's context menu acts on that chat without changing the selection (#491)", async () => {
    const { page, errors } = await harness.open();
    const composer = page.locator("textarea.composer-input");
    const selected = await composer.getAttribute("aria-label");
    const researcher = page.getByRole("option", { name: /Researcher/ });
    await researcher.click({ button: "right" });
    const items = page.locator(".page-menu-item");
    await expect.poll(() => items.allInnerTexts()).toEqual(expect.arrayContaining(["Pin", "Change Look…"]));
    // Right-clicking did not select the row.
    expect(await composer.getAttribute("aria-label")).toBe(selected);
    await items.filter({ hasText: "Pin" }).first().click();
    await expect.poll(() => researcher.getByRole("img", { name: "Pinned" }).count()).toBe(1);
    expect(await composer.getAttribute("aria-label")).toBe(selected);
    expect(errors).toEqual([]);
    await page.close();
  });

  test("Ctrl+B hides and shows the sidebar", async () => {
    const { page } = await harness.open();
    const list = page.getByRole("listbox").first();
    await page.keyboard.press("Control+B");
    await expect.poll(() => list.isVisible()).toBe(false);
    await page.keyboard.press("Control+B");
    await expect.poll(() => list.isVisible()).toBe(true);
    await page.close();
  });
});
