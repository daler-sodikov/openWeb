import { describe, expect, it } from "vitest";

import type { TestStatus, TestTreeNode } from "../ipc/bindings";
import { exitText, formatDuration, nodeKey, summaryText, visibleRows } from "./format";

function node(
  kind: TestTreeNode["kind"],
  name: string,
  status: TestStatus,
  children: TestTreeNode[] = [],
  fullName: string[] = kind === "file" ? [] : [name],
): TestTreeNode {
  return {
    kind,
    name,
    fullName,
    status,
    durationMs: null,
    location: { path: "src/a.test.ts", inProject: true, line: 0, column: 0 },
    failures: [],
    children,
  };
}

const tree = [
  node("file", "src/a.test.ts", "failed", [
    node(
      "suite",
      "math",
      "failed",
      [
        node("test", "adds", "passed", [], ["math", "adds"]),
        node("test", "divides", "failed", [], ["math", "divides"]),
        node("test", "later", "skipped", [], ["math", "later"]),
      ],
      ["math"],
    ),
  ]),
];

const names = (rows: ReturnType<typeof visibleRows>) =>
  rows.map((r) => `${String(r.depth)}:${r.node.name}`);

describe("visibleRows", () => {
  it("lists everything with depths", () => {
    const all = new Set<TestStatus>(["passed", "failed", "skipped"]);
    expect(names(visibleRows(tree, all, new Set()))).toEqual([
      "0:src/a.test.ts",
      "1:math",
      "2:adds",
      "2:divides",
      "2:later",
    ]);
  });

  it("keeps only matching tests and their parents", () => {
    expect(names(visibleRows(tree, new Set(["failed"]), new Set()))).toEqual([
      "0:src/a.test.ts",
      "1:math",
      "2:divides",
    ]);
    expect(visibleRows(tree, new Set(), new Set())).toEqual([]);
  });

  it("hides the children of collapsed nodes", () => {
    const suite = tree[0]?.children[0];
    if (!suite) throw new Error("fixture");
    const rows = visibleRows(tree, new Set(["passed", "failed"]), new Set([nodeKey(suite)]));
    expect(names(rows)).toEqual(["0:src/a.test.ts", "1:math"]);
    expect(rows[1]?.collapsed).toBe(true);
    expect(rows[1]?.hasChildren).toBe(true);
  });
});

describe("texts", () => {
  it("formats durations", () => {
    expect(formatDuration(null)).toBe("");
    expect(formatDuration(12.4)).toBe("12 ms");
    expect(formatDuration(1500)).toBe("1.5 s");
    expect(formatDuration(125_000)).toBe("2 min 5 s");
  });

  it("says how a run ended", () => {
    expect(exitText(0, 12)).toBe("Exit code 0 · 12 ms");
    expect(exitText(null, 12)).toBe("Killed · 12 ms");
    expect(exitText(1, 1500, true)).toBe("Stopped · Exit code 1 · 1.5 s");
    expect(exitText(null, null, true)).toBe("Stopped");
  });

  it("summarizes test counts", () => {
    expect(summaryText([3, 1, 1])).toBe("1 failed, 3 passed, 1 skipped of 5 tests");
    expect(summaryText([1, 0, 0])).toBe("1 passed of 1 test");
  });
});
