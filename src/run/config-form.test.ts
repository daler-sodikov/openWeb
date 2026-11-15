import { describe, expect, it } from "vitest";

import type { RunConfig } from "../ipc/bindings";
import { emptyConfig, fromForm, toForm } from "./config-form";

const vitest: RunConfig = {
  id: "cfg-1",
  name: "Unit",
  kind: {
    type: "test",
    framework: "vitest",
    scope: { files: ["src/a.test.ts"], tests: [{ file: "src/b.test.ts", name: ["b"], line: 3 }] },
  },
  cwd: "packages/app",
  args: ["--coverage"],
  env: [["CI", "1"]],
  packageManager: null,
};

describe("run configuration form", () => {
  it("round-trips a configuration and keeps its single tests", () => {
    const form = toForm(vitest);
    expect(form.kind).toBe("vitest");
    expect(form.files).toBe("src/a.test.ts");
    expect(form.env).toBe("CI=1");
    expect(fromForm(form, vitest)).toEqual({ config: vitest });
  });

  it("reads one argument and one variable per line", () => {
    const form = {
      ...toForm(emptyConfig()),
      name: " Dev ",
      target: "dev",
      args: "--port\n 3000 \n\n",
      env: "A=b=c\nEMPTY=",
      packageManager: "pnpm" as const,
    };
    expect(fromForm(form, null)).toEqual({
      config: {
        id: "",
        name: "Dev",
        kind: { type: "npmScript", script: "dev" },
        cwd: "",
        args: ["--port", "3000"],
        env: [
          ["A", "b=c"],
          ["EMPTY", ""],
        ],
        packageManager: "pnpm",
      },
    });
  });

  it("says what is missing or malformed", () => {
    const base = toForm(emptyConfig());
    expect(fromForm({ ...base, target: "dev" }, null)).toEqual({ error: "Enter a name." });
    expect(fromForm({ ...base, name: "x" }, null)).toEqual({ error: "Enter the script to run." });
    expect(fromForm({ ...base, name: "x", kind: "node" }, null)).toEqual({
      error: "Enter the file to run.",
    });
    expect(fromForm({ ...base, name: "x", target: "dev", env: "NOEQUALS" }, null)).toEqual({
      error: 'Environment: write NAME=value (not "NOEQUALS").',
    });
  });

  it("drops the package manager for anything but scripts", () => {
    const form = { ...toForm(emptyConfig()), name: "n", kind: "node" as const, target: "a.js" };
    const result = fromForm({ ...form, packageManager: "bun" }, null);
    expect("config" in result ? result.config.packageManager : "error").toBe(null);
  });
});
