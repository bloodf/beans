import { beforeEach, expect, vi, test } from "vitest";

// Render the production dialog through native component tags and a state-hook boundary.
// Android window presentation, TalkBack and system confirmation still need device evidence.
type Node = { type: any; props: any };
let platform = "android";
let hidden: string | null = null;
let state = { status: "downloading", progress: 0.25, installingVersion: "1.10.0", installingId: "consent-1" };
const jsx = (type: any, props: any): Node => ({ type, props });
vi.doMock("react", () => ({ useState: () => [hidden, (next: string | null) => { hidden = next; }] }));
vi.doMock("react/jsx-runtime", () => ({ jsx, jsxs: jsx }));
vi.doMock("react/jsx-dev-runtime", () => ({ jsxDEV: jsx }));
vi.doMock("react-native", () => ({ Platform: { get OS() { return platform; } }, StyleSheet: { absoluteFill: {} } }));
const dialog = Object.assign(() => null, { Title: "Title", Text: "DialogText", ConfirmButton: "ConfirmButton" });
vi.doMock("@expo/ui/jetpack-compose", () => ({ AlertDialog: dialog, Column: "Column", Host: "Host", LinearProgressIndicator: "Progress", Spacer: "Spacer", Text: "Text", TextButton: "Button" }));
vi.doMock("@expo/ui/jetpack-compose/modifiers", () => ({ fillMaxWidth: () => ({}), height: () => ({}) }));
vi.doMock("../core/updates", () => ({
  useUpdates: (select: (value: typeof state) => unknown) => select(state),
  isUpdateProgressActive: (status: string) => ["downloading", "installing", "confirmation"].includes(status),
}));
vi.doMock("../i18n", () => ({ t: (text: string, values: Record<string, unknown> = {}) => text.replace(/\{(\w+)\}/g, (_, name) => String(values[name])) }));
vi.doMock("./theme", () => ({ usePalette: () => ({ cell: "cell", label: "label", secondaryLabel: "secondary", tint: "tint" }) }));
const { UpdateProgress } = await import("./UpdateProgress");
const find = (node: Node | null, type: any): Node | undefined => {
  if (!node || typeof node !== "object") return;
  if (node.type === type) return node;
  return [node.props.children].flat().map(child => find(child, type)).find(Boolean);
};
beforeEach(() => { platform = "android"; hidden = null; state = { status: "downloading", progress: 0.25, installingVersion: "1.10.0", installingId: "consent-1" }; });

test("download percent and indeterminate installer phases use the current attempt", () => {
  expect(find(UpdateProgress(), "Progress")!.props.progress).toBe(0.25);
  for (const status of ["installing", "confirmation"]) {
    state.status = status;
    expect(find(UpdateProgress(), "Progress")!.props.progress).toBeNull();
  }
  state.status = "error";
  expect(UpdateProgress()).toBeNull();
});

test("Hide and Back hide only their attempt, leaving its download state intact", () => {
  const first = UpdateProgress();
  find(first, "Button")!.props.onClick();
  expect(UpdateProgress()).toBeNull();
  expect(state).toMatchObject({ status: "downloading", progress: 0.25 });
  // No intervening idle render: a retry still opens the dialog.
  state.installingId = "consent-2";
  expect(UpdateProgress()).not.toBeNull();
  find(UpdateProgress(), dialog)!.props.onDismissRequest();
  expect(UpdateProgress()).toBeNull();
  expect(state.status).toBe("downloading");
});

test("iOS, blocked and missing-attempt state never present an update dialog", () => {
  platform = "ios";
  expect(UpdateProgress()).toBeNull();
  platform = "android"; state.status = "blocked";
  expect(UpdateProgress()).toBeNull();
  state.status = "downloading"; state.installingId = "";
  expect(UpdateProgress()).toBeNull();
});
