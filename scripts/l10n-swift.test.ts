import { expect, test } from "bun:test";
import { memorySwiftKeys } from "./l10n-swift";
const path = "Settings/MemoryEmbeddingViewController.swift";

test("memory helpers expose only display arguments, including choices and placeholders", () => {
  const keys = memorySwiftKeys(`
    row("Endpoint", endpoint); text("Name", value: "private value", placeholder: "Enter name");
    popup("Distance", choices: ["cosine", "dot"]); note("Costs, limits (and scope)");
    Field("wire_id", "Display ID", required: true); MemoryActionButton("Save") {}
  `, path);
  expect([...keys].sort()).toEqual(["Endpoint", "Name", "Enter name", "Distance", "cosine", "dot", "Costs, limits (and scope)", "Display ID", "Save"].sort());
  expect(keys.has("private value")).toBe(false);
  expect(keys.has("wire_id")).toBe(false);
});

test("conditional labels include results but never comparison constants", () => {
  const keys = memorySwiftKeys(`L(kind == "not a label" ? "New" : "Edit"); confirm(clear ? "Clear" : "Delete", details: "already translated elsewhere")`, path);
  expect([...keys].sort()).toEqual(["New", "Edit", "Clear", "Delete"].sort());
});

test("truly unused entries, comments, arbitrary variables and unrelated helpers stay unused", () => {
  const source = `// note("Comment")
    /* row("Old", field) */
    let unused = "Unused"; note(runtimeValue); output("Untranslated output");
    text("Actual label", value: "not localized");`;
  const keys = memorySwiftKeys(source, path);
  expect([...keys]).toEqual(["Actual label"]);
  expect(["Unused", "Comment", "Old"].filter(key => !keys.has(key))).toEqual(["Unused", "Comment", "Old"]);
  expect([...memorySwiftKeys(source, "Unrelated.swift")]).toEqual([]);
});

test("quoted punctuation and nested conditions do not leak non-label values", () => {
  const keys = memorySwiftKeys(`text("Name, with (punctuation)", value: choose("secret", nil)); L(a ? (b ? "Inner" : "Other") : "Outer")`, path);
  expect(keys.has("Name, with (punctuation)")).toBe(true);
  expect(keys.has("secret")).toBe(false);
  expect([...keys].sort()).toEqual(["Name, with (punctuation)", "Inner", "Other", "Outer"].sort());
});

test("escaped quotes and newlines belong to the literal label", () => {
  const source = String.raw`note("Say \"hello\"\nnow"); row("Private path", file("not a key"));`;
  expect([...memorySwiftKeys(source, path)]).toEqual(['Say "hello"\nnow', "Private path"]);
});


test("nested comments and quoted source samples never count as calls", () => {
  const source = String.raw`/* outer /* inner */ note("Not used") */
    let sample = """note("Also unused")"""
    let raw = #"note("Raw unused")"#
    note("Used")`;
  expect([...memorySwiftKeys(source, path)]).toEqual(["Used"]);
});

test("parentheses around a condition do not hide either label branch", () => {
  expect([...memorySwiftKeys('note((enabled) ? "Shown A" : ("Shown B"))', path)])
    .toEqual(["Shown A", "Shown B"]);
});
