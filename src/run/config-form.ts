// The run configuration editor's fields and their conversion to and from
// the configuration Rust stores. Form handling only: Rust checks and runs.
import type { PackageManager, RunConfig, TestFramework } from "../ipc/bindings";

export type FormKind = "node" | "npmScript" | TestFramework;

export interface ConfigForm {
  name: string;
  kind: FormKind;
  /** The file (Node) or script (npm script); unused for tests. */
  target: string;
  /** Test files, one per line. */
  files: string;
  cwd: string;
  /** One argument per line (no shell quoting). */
  args: string;
  /** `NAME=value`, one per line. */
  env: string;
  /** "" = detected. */
  packageManager: PackageManager | "";
}

export const KIND_LABELS: Record<FormKind, string> = {
  node: "Node.js",
  npmScript: "npm script",
  vitest: "Vitest",
  jest: "Jest",
  playwright: "Playwright",
};

function lines(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
}

export function toForm(config: RunConfig): ConfigForm {
  const kind = config.kind;
  return {
    name: config.name,
    kind: kind.type === "test" ? kind.framework : kind.type,
    target: kind.type === "node" ? kind.file : kind.type === "npmScript" ? kind.script : "",
    files: kind.type === "test" ? kind.scope.files.join("\n") : "",
    cwd: config.cwd,
    args: config.args.join("\n"),
    env: config.env.map(([k, v]) => `${k}=${v}`).join("\n"),
    packageManager: config.packageManager ?? "",
  };
}

/**
 * The configuration for `form`, keeping `base`'s id and single tests, or
 * what is wrong with the form.
 */
export function fromForm(
  form: ConfigForm,
  base: RunConfig | null,
): { config: RunConfig } | { error: string } {
  const env: [string, string][] = [];
  for (const line of lines(form.env)) {
    const eq = line.indexOf("=");
    if (eq <= 0) return { error: `Environment: write NAME=value (not "${line}").` };
    env.push([line.slice(0, eq).trim(), line.slice(eq + 1)]);
  }
  const target = form.target.trim();
  let kind: RunConfig["kind"];
  switch (form.kind) {
    case "node":
      if (!target) return { error: "Enter the file to run." };
      kind = { type: "node", file: target };
      break;
    case "npmScript":
      if (!target) return { error: "Enter the script to run." };
      kind = { type: "npmScript", script: target };
      break;
    default: {
      const tests =
        base?.kind.type === "test" && base.kind.framework === form.kind
          ? base.kind.scope.tests
          : [];
      kind = { type: "test", framework: form.kind, scope: { files: lines(form.files), tests } };
    }
  }
  if (!form.name.trim()) return { error: "Enter a name." };
  return {
    config: {
      id: base?.id ?? "",
      name: form.name.trim(),
      kind,
      cwd: form.cwd.trim(),
      args: lines(form.args),
      env,
      packageManager:
        form.kind === "npmScript" && form.packageManager !== "" ? form.packageManager : null,
    },
  };
}

/** A new configuration's starting point. */
export function emptyConfig(): RunConfig {
  return {
    id: "",
    name: "",
    kind: { type: "npmScript", script: "" },
    cwd: "",
    args: [],
    env: [],
    packageManager: null,
  };
}
