import { expect, test } from "bun:test";
import { join } from "node:path";

const ROOT = join(import.meta.dir, "..");
const workflows = await Promise.all(["test.yml", "docs.yml", "catalog.yml"].map(async (file) => ({
  file,
  workflow: Bun.YAML.parse(await Bun.file(join(ROOT, ".github/workflows", file)).text()),
})));

function evaluate(expression: string, github: object, env: object = {}) {
  const body = expression.replace(/^\s*\$\{\{\s*|\s*\}\}\s*$/g, "");
  return new Function("github", "env", `return (${body});`)(github, env);
}

for (const { file, workflow } of workflows) {
  test(`${file}: PRs targeting main and manual dispatch trigger validation`, () => {
    expect(workflow.on).toEqual({ pull_request: { branches: ["main"] }, workflow_dispatch: null });
  });

  test(`${file}: any same-repository PR and manual dispatch allocate runners, forks do not`, () => {
    for (const [name, job] of Object.entries(workflow.jobs) as [string, { if: string }][]) {
      for (const [head, repo, expected] of [
        ["release/1.0.15", "bloodf/beans", true],
        ["release/integration/1.0.15", "bloodf/beans", true],
        ["feature/chat", "bloodf/beans", true],
        ["fleet/r20261014a-ci", "bloodf/beans", true],
        ["main", "bloodf/beans", true],
        ["release-fix", "bloodf/beans", true],
        ["release/1.0.15", "contributor/beans", false],
        ["feature/chat", "contributor/beans", false],
      ] as const) {
        const github = {
          repository: "bloodf/beans",
          event_name: "pull_request",
          head_ref: head,
          event: { pull_request: { head: { repo: { full_name: repo } } } },
        };
        expect(evaluate(job.if, github), `${file} ${name}: ${repo}:${head}`).toBe(expected);
      }
      // Dispatch has no pull_request payload; short-circuit admission must not dereference it.
      expect(evaluate(job.if, { repository: "bloodf/beans", event_name: "workflow_dispatch", event: {} }), `${file} ${name}: dispatch`).toBe(true);
    }
  });
}

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


test("every Rust aggregate receives a prebuilt real relay for the revocation fixture", () => {
  const jobs = workflows.find(({ file }) => file === "test.yml")!.workflow.jobs;
  for (const name of ["rust-linux", "rust-macos", "rust-windows"]) {
    const steps = jobs[name].steps as { run?: string; if?: string }[];
    const build = steps.findIndex((step) => step.run?.includes("cargo build --locked -p beans-relay --bin beans-relay"));
    const aggregate = steps.findIndex((step) => step.run?.includes("cargo test --locked"));
    expect(build, `${name}: build relay before tests`).toBeGreaterThanOrEqual(0);
    expect(build).toBeLessThan(aggregate);
    expect(steps[build].if).toBeUndefined();
    expect(steps[build].run).toContain("BEANS_REVOCATION_RELAY_BIN=");
    expect(steps[build].run).toContain("GITHUB_ENV");
    expect(steps[build].run).toContain("cargo metadata --locked --no-deps --format-version 1");
  }
});
