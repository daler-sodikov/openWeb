import { describe, expect, it } from "vitest";

import type { GitBranch } from "../ipc/bindings";
import { branchItems } from "./branches-popup";

const b = (name: string, more: Partial<GitBranch> = {}): GitBranch => ({
  name,
  remote: false,
  current: false,
  upstream: null,
  subject: "",
  ...more,
});

describe("branchItems", () => {
  it("lists New Branch, the current, local, then remote branches, filtered", () => {
    const branches = [
      b("origin/main", { remote: true }),
      b("zeta"),
      b("main", { current: true }),
      b("alpha"),
    ];
    const names = (q: string) =>
      branchItems(branches, q).map((i) => (i.kind === "new" ? "+" : i.branch.name));
    expect(names("")).toEqual(["+", "main", "alpha", "zeta", "origin/main"]);
    expect(names("MAI")).toEqual(["+", "main", "origin/main"]);
  });
});
