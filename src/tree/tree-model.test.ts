import { describe, expect, it } from "vitest";

import type { DirEntry } from "../ipc/bindings";
import { flattenTree, visibleRange } from "./tree-model";

const dir = (path: string): DirEntry => ({
  name: path.split("/").pop() ?? path,
  path,
  kind: "directory",
});
const file = (path: string): DirEntry => ({
  name: path.split("/").pop() ?? path,
  path,
  kind: "file",
});

describe("flattenTree", () => {
  const root = [dir("src"), dir("docs"), file("README.md")];
  const children = new Map<string, DirEntry[]>([
    ["src", [dir("src/lib"), file("src/main.ts")]],
    ["src/lib", [file("src/lib/a.ts")]],
  ]);

  it("shows only the root when nothing is expanded", () => {
    expect(flattenTree(root, children, new Set()).map((r) => r.entry.path)).toEqual([
      "src",
      "docs",
      "README.md",
    ]);
  });

  it("nests expanded, loaded directories with depth", () => {
    const rows = flattenTree(root, children, new Set(["src", "src/lib", "docs"]));
    expect(rows.map((r) => [r.entry.path, r.depth, r.expanded])).toEqual([
      ["src", 0, true],
      ["src/lib", 1, true],
      ["src/lib/a.ts", 2, false],
      ["src/main.ts", 1, false],
      ["docs", 0, true], // expanded but not loaded yet
      ["README.md", 0, false],
    ]);
  });

  it("hides children of collapsed parents", () => {
    const rows = flattenTree(root, children, new Set(["src/lib"]));
    expect(rows.map((r) => r.entry.path)).toEqual(["src", "docs", "README.md"]);
  });
});

describe("visibleRange", () => {
  it("covers the viewport plus overscan, clamped", () => {
    expect(visibleRange(0, 100, 20, 1000, 2)).toEqual({ start: 0, end: 7 });
    expect(visibleRange(2000, 100, 20, 1000, 2)).toEqual({ start: 98, end: 107 });
    expect(visibleRange(19_990, 100, 20, 1000, 2)).toEqual({ start: 997, end: 1000 });
    expect(visibleRange(0, 100, 20, 0)).toEqual({ start: 0, end: 0 });
  });
});
