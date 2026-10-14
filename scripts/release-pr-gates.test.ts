import { expect, test } from "bun:test";
import { join } from "node:path";

const ROOT = join(import.meta.dir, "..");
const workflows = await Promise.all(["test.yml", "docs.yml", "catalog.yml"].map(async (file) => ({
  file,
  workflow: Bun.YAML.parse(await Bun.file(join(ROOT, ".github/workflows", file)).text()),
})));

function evaluate(expression: string, github: object, env: object = {}, inputs: object = {}) {
  const body = expression.replace(/^\s*\$\{\{\s*|\s*\}\}\s*$/g, "");
  return new Function("github", "env", "inputs", `return (${body});`)(github, env, inputs);
}

for (const { file, workflow } of workflows) {
  test(`${file}: validation triggers admit main PRs without path filters`, () => {
    expect(workflow.on.pull_request).toEqual({ branches: ["main"] });
    expect(Object.hasOwn(workflow.on, "workflow_dispatch")).toBe(true);
  });

  test(`${file}: runner admission follows event and platform scope`, () => {
    for (const [name, job] of Object.entries(workflow.jobs) as [string, { if: string }][]) {
      const deferred = file === "test.yml" && ["rust-windows", "desktop", "mobile"].includes(name);
      for (const repo of ["bloodf/beans", "contributor/beans"]) {
        const github = {
          repository: "bloodf/beans",
          event_name: "pull_request",
          event: { pull_request: { head: { repo: { full_name: repo } } } },
        };
        expect(Boolean(evaluate(job.if, github)), `${file} ${name}: ${repo}`).toBe(!deferred && repo === "bloodf/beans");
      }
      const dispatch = { repository: "bloodf/beans", event_name: "workflow_dispatch", event: {} };
      for (const full_platform of [undefined, false, true]) {
        expect(Boolean(evaluate(job.if, dispatch, {}, { full_platform })), `${file} ${name}: ${full_platform}`).toBe(!deferred || full_platform === true);
      }
      if (file === "test.yml") {
        expect(Boolean(evaluate(job.if, { repository: "bloodf/beans", event_name: "pull_request", event: {} }))).toBe(false);
        expect(Boolean(evaluate(job.if, { repository: "bloodf/beans", event_name: "push", event: {} }))).toBe(false);
      }
    }
  });
}

test("manual full-platform input is a native boolean, defaulting to server/AppKit", () => {
  const workflow = workflows.find(({ file }) => file === "test.yml")!.workflow;
  const input = workflow.on.workflow_dispatch.inputs.full_platform;
  expect(input.type).toBe("boolean");
  expect(input.default).toBe(false);
  for (const name of ["rust-linux", "rust-macos"]) {
    const job = workflow.jobs[name];
    const workspace = job.steps.find((step: { run?: string }) => step.run === "cargo test --locked --workspace --exclude beans-mobile");
    expect(workspace.if).toBeUndefined();
    const phone = job.steps.find((step: { run?: string }) => step.run === "cargo test --locked -p beans-mobile");
    for (const full_platform of [undefined, false, true]) {
      expect(Boolean(evaluate(phone.if, { event_name: "workflow_dispatch" }, {}, { full_platform }))).toBe(full_platform === true);
      expect(Boolean(evaluate(phone.if, { event_name: "pull_request" }, {}, { full_platform }))).toBe(false);
    }
  }
  expect(workflow.jobs["rust-linux"].steps.some((step: { run?: string }) => step.run === "python3 -I -B scripts/server-updater.test.py -v")).toBe(true);
  expect(workflow.jobs["rust-linux"].services.postgres.image).toBe("postgres:17");
  expect(workflow.jobs["macos-app"].steps.filter((step: { run?: string }) => step.run).map((step: { run: string }) => step.run)).toEqual([
    'bun -e \'import { buildMarkdown } from "./scripts/app.ts"; if (!(await buildMarkdown("debug")).ok) process.exit(1)\'',
    "swift test", "bun run test:mac-startup", "sh macos/Tests/run-settings-layout.sh",
  ]);
});

test("workflow check names are unique across validation workflows", () => {
  const names = workflows.flatMap(({ workflow }) => Object.entries(workflow.jobs).map(([id, job]) => {
    if (job && typeof job === "object" && "name" in job && typeof job.name === "string") return job.name;
    return id;
  }));
  expect(new Set(names).size).toBe(names.length);
  expect(names).toContain("docs");
  expect(names).toContain("catalog");
});

test("catalog uses PR base or main for dispatch in both fetch and comparison", () => {
  const job = workflows.find(({ file }) => file === "catalog.yml")!.workflow.jobs.catalog;
  for (const [github, expected] of [
    [{ event_name: "pull_request", base_ref: "main" }, "main"],
    [{ event_name: "pull_request", base_ref: "integration/catalog" }, "integration/catalog"],
    [{ event_name: "workflow_dispatch", base_ref: "" }, "main"],
    [{ event_name: "workflow_dispatch" }, "main"],
  ] as const) {
    const env = { CATALOG_BASE_REF: evaluate(job.env.CATALOG_BASE_REF, github) };
    expect(env.CATALOG_BASE_REF).toBe(expected);
    const fetch = job.steps.find((step: { run?: string }) => step.run?.startsWith("git fetch ")).run;
    expect(fetch.replaceAll("$CATALOG_BASE_REF", env.CATALOG_BASE_REF)).toBe(`git fetch --depth=1 origin "+refs/heads/${expected}:refs/remotes/origin/${expected}"`);
    const check = job.steps.find((step: { run?: string }) => step.run === "bun run check:catalog");
    const base = check.env.CATALOG_BASE.replace(/\$\{\{(.*?)\}\}/g, (_: string, expression: string) => evaluate(expression, github, env));
    expect(base).toBe(`origin/${expected}`);
  }
});
