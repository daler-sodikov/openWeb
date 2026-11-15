import { describe, expect, it } from "vitest";

import type { GitStatus } from "../ipc/bindings";
import { branchLabel, GitStatusModel, isNewer } from "./status";

const status: GitStatus = {
  unavailable: null,
  branch: "main",
  head: "abc12345",
  upstream: null,
  ahead: 0,
  behind: 0,
  files: [
    { path: "src/a.ts", origPath: null, kind: "modified", staged: false, unstaged: true },
    { path: "new.ts", origPath: null, kind: "untracked", staged: false, unstaged: true },
    { path: "debug.log", origPath: null, kind: "ignored", staged: false, unstaged: false },
  ],
  changedDirs: ["src"],
  ignoredDirs: ["node_modules", "dist"],
  generation: 3,
};

describe("GitStatusModel", () => {
  it("gives files their kind, folders below ignored ones ignored, and changed folders", () => {
    const model = new GitStatusModel();
    model.update(status);
    expect(model.classFor("src/a.ts", false)).toBe("modified");
    expect(model.classFor("new.ts", false)).toBe("untracked");
    expect(model.classFor("debug.log", false)).toBe("ignored");
    expect(model.classFor("node_modules", true)).toBe("ignored");
    expect(model.classFor("node_modules/react/index.js", false)).toBe("ignored");
    expect(model.classFor("distant.ts", false)).toBeNull();
    expect(model.classFor("src", true)).toBe("changed");
    expect(model.classFor("src/b.ts", false)).toBeNull();
    expect(model.classFor("public", true)).toBeNull();
  });
});

describe("which status the view keeps", () => {
  const base: GitStatus = {
    unavailable: null,
    branch: "main",
    head: "8fe666af",
    upstream: null,
    ahead: 0,
    behind: 0,
    files: [],
    changedDirs: [],
    ignoredDirs: [],
    generation: 7,
  };
  const looking: GitStatus = {
    ...base,
    unavailable: "Looking for a Git repository…",
    branch: null,
    head: null,
    generation: 6,
  };

  it("keeps the newest, whichever arrives last", () => {
    // The event of the real status came first; the answer to the earlier
    // question (still "looking") arrives after it and must not win.
    expect(isNewer(-1, base)).toBe(true);
    expect(isNewer(base.generation, looking)).toBe(false);
    expect(isNewer(base.generation, base)).toBe(false);
    expect(isNewer(looking.generation, base)).toBe(true);
  });

  it("shows a branch, a detached commit, or nothing", () => {
    expect(branchLabel(base)).toBe("main");
    expect(branchLabel({ ...base, branch: null })).toBe("detached 8fe666af");
    expect(branchLabel({ ...base, branch: null, head: null })).toBe("");
    expect(branchLabel(looking)).toBe("");
  });
});
