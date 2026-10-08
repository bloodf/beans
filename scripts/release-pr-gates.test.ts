import { expect, test } from "bun:test";
import { join } from "node:path";

const ROOT = join(import.meta.dir, "..");

for (const file of ["test.yml", "docs.yml", "catalog.yml"]) {
  test(`${file}: only same-repository release PRs targeting main allocate runners`, async () => {
    const workflow = Bun.YAML.parse(await Bun.file(join(ROOT, ".github/workflows", file)).text());
    expect(workflow.on).toEqual({ pull_request: { branches: ["main"] } });
    for (const [name, job] of Object.entries(workflow.jobs) as [string, { if?: string }][]) {
      expect(job.if, `${file} ${name}: job-level gate`).toBeDefined();
      // Evaluate the workflow expression, not a separately copied predicate. These gates use
      // JavaScript-compatible operators; GitHub's startsWith ignores case.
      const expression = job.if!.replace(/^\s*\$\{\{\s*|\s*\}\}\s*$/g, "");
      const allows = new Function("github", "startsWith", `return (${expression});`);
      for (const [head, repo, expected] of [
        ["release/1.0.15", "bloodf/beans", true],
        ["release/integration/1.0.15", "bloodf/beans", true],
        ["feature/chat", "bloodf/beans", false],
        ["main", "bloodf/beans", false],
        ["release-fix", "bloodf/beans", false],
        ["release/1.0.15", "contributor/beans", false],
        ["feature/chat", "contributor/beans", false],
      ] as const) {
        const github = { repository: "bloodf/beans", head_ref: head, event: { pull_request: { head: { repo: { full_name: repo } } } } };
        expect(allows(github, (value: string, prefix: string) => value.toLowerCase().startsWith(prefix.toLowerCase())), `${file} ${name}: ${repo}:${head}`).toBe(expected);
      }
    }
  });
}
