// Problems tool window: problems from the language server grouped by file
// (virtualized), plus the counts shown in the status bar.
import type { FileProblems, Problem, Severity } from "../ipc/bindings";
import { bindListKeys } from "../popup/popup";
import { VirtualList } from "../ui/virtual-list";

export type ProblemRow =
  | { kind: "file"; path: string; count: number }
  | { kind: "problem"; path: string; problem: Problem };

const ORDER: Record<Severity, number> = { error: 0, warning: 1, information: 2, hint: 3 };

/** Rows grouped by file (sorted by path), problems by severity then position. */
export function problemRows(files: ReadonlyMap<string, readonly Problem[]>): ProblemRow[] {
  const rows: ProblemRow[] = [];
  for (const path of [...files.keys()].sort()) {
    const problems = [...(files.get(path) ?? [])].sort(
      (a, b) => ORDER[a.severity] - ORDER[b.severity] || a.line - b.line || a.column - b.column,
    );
    if (problems.length === 0) continue;
    rows.push({ kind: "file", path, count: problems.length });
    for (const problem of problems) rows.push({ kind: "problem", path, problem });
  }
  return rows;
}

/** Errors, warnings and the rest (information and hints). */
export function problemCounts(files: ReadonlyMap<string, readonly Problem[]>): {
  errors: number;
  warnings: number;
  other: number;
} {
  const counts = { errors: 0, warnings: 0, other: 0 };
  for (const problems of files.values()) {
    for (const p of problems) {
      if (p.severity === "error") counts.errors++;
      else if (p.severity === "warning") counts.warnings++;
      else counts.other++;
    }
  }
  return counts;
}

const ICONS: Record<Severity, string> = { error: "✖", warning: "▲", information: "ℹ", hint: "•" };

export class ProblemsPanel {
  readonly element: HTMLElement;
  private readonly title = document.createElement("span");
  private readonly empty = document.createElement("div");
  private readonly list: VirtualList<ProblemRow>;
  private readonly files = new Map<string, readonly Problem[]>();
  /** Called whenever the problems change (status bar counts). */
  onChange: (() => void) | null = null;

  constructor(private readonly openLocation: (path: string, line: number, column: number) => void) {
    this.element = document.createElement("section");
    this.element.className = "tool-window problems";
    this.element.dataset.testid = "problems";
    this.element.setAttribute("aria-label", "Problems");
    this.element.hidden = true;
    this.element.tabIndex = -1;
    const header = document.createElement("div");
    header.className = "tool-window-header";
    this.title.className = "tool-window-title";
    const close = document.createElement("button");
    close.type = "button";
    close.className = "tool-window-close";
    close.textContent = "×";
    close.setAttribute("aria-label", "Hide Problems");
    close.addEventListener("click", () => {
      this.toggle(false);
    });
    header.append(this.title, close);
    this.list = new VirtualList<ProblemRow>(
      22,
      (row) => this.renderRow(row),
      (row) => {
        if (row.kind === "problem") {
          this.openLocation(row.path, row.problem.line, row.problem.column);
        } else {
          this.openLocation(row.path, 0, 0);
        }
      },
      "problem-list",
    );
    this.empty.className = "tool-window-empty";
    this.empty.textContent = "No problems found.";
    this.element.append(header, this.list.element, this.empty);
    bindListKeys(this.element, this.list);
    this.render();
  }

  get visible(): boolean {
    return !this.element.hidden;
  }

  toggle(show = !this.visible): void {
    this.element.hidden = !show;
    if (show) {
      this.render();
      this.element.focus();
    }
  }

  setAll(files: readonly FileProblems[]): void {
    this.files.clear();
    for (const file of files) this.files.set(file.path, file.problems);
    this.render();
  }

  update(file: FileProblems): void {
    if (file.problems.length === 0) this.files.delete(file.path);
    else this.files.set(file.path, file.problems);
    this.render();
  }

  counts(): ReturnType<typeof problemCounts> {
    return problemCounts(this.files);
  }

  private render(): void {
    const rows = problemRows(this.files);
    const total = rows.filter((r) => r.kind === "problem").length;
    this.title.textContent = total > 0 ? `Problems (${String(total)})` : "Problems";
    this.empty.hidden = total > 0;
    if (this.visible) this.list.setItems(rows);
    this.onChange?.();
  }

  private renderRow(row: ProblemRow): HTMLElement {
    const el = document.createElement("div");
    if (row.kind === "file") {
      el.className = "problem-file";
      el.dataset.testid = "problem-file";
      el.dataset.path = row.path;
      const name = document.createElement("span");
      name.className = "problem-file-name";
      name.textContent = row.path;
      const count = document.createElement("span");
      count.className = "problem-count";
      count.textContent = `${String(row.count)} ${row.count === 1 ? "problem" : "problems"}`;
      el.append(name, count);
      return el;
    }
    const { problem } = row;
    el.className = `problem-item severity-${problem.severity}`;
    el.dataset.testid = "problem-item";
    el.dataset.severity = problem.severity;
    const icon = document.createElement("span");
    icon.className = "problem-icon";
    icon.textContent = ICONS[problem.severity];
    icon.setAttribute("aria-label", problem.severity);
    const message = document.createElement("span");
    message.className = "problem-message";
    message.textContent = problem.message;
    const where = document.createElement("span");
    where.className = "problem-location";
    where.textContent = `${String(problem.line + 1)}:${String(problem.column + 1)}`;
    el.title = [problem.source, problem.code].filter((p) => p !== null).join(" ");
    el.append(icon, message, where);
    return el;
  }
}
