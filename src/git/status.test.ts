import { describe, expect, it } from "vitest";

import type { GitStatus } from "../ipc/bindings";
import { GitStatusModel } from "./status";

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
