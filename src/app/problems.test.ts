import { describe, expect, it } from "vitest";

import type { Problem, Severity } from "../ipc/bindings";
import { problemCounts, problemRows } from "./problems";

const problem = (severity: Severity, line: number, message = "m"): Problem => ({
  severity,
  message,
  source: "typescript",
  code: "2322",
  line,
  column: 0,
  endLine: line,
  endColumn: 1,
});

describe("problemRows", () => {
  it("groups by file (sorted) and orders by severity, then position", () => {
    const files = new Map([
      ["src/b.ts", [problem("warning", 1), problem("error", 9), problem("error", 2)]],
      ["src/a.ts", [problem("hint", 0)]],
      ["src/empty.ts", []],
    ]);
    const rows = problemRows(files);
    expect(
      rows.map((r) => (r.kind === "file" ? `${r.path} (${String(r.count)})` : r.problem.line)),
    ).toEqual(["src/a.ts (1)", 0, "src/b.ts (3)", 2, 9, 1]);
  });
});

describe("problemCounts", () => {
  it("counts errors, warnings and the rest", () => {
    const files = new Map([
      ["a", [problem("error", 0), problem("warning", 1), problem("information", 2)]],
      ["b", [problem("error", 0), problem("hint", 1)]],
    ]);
    expect(problemCounts(files)).toEqual({ errors: 2, warnings: 1, other: 2 });
  });
});
