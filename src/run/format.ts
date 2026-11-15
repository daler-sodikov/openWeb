// Display helpers for runs and test results: status glyphs, durations,
// summaries and the rows of the test tree. View formatting only; the
// results themselves come from Rust.
import type { TestStatus, TestTreeNode } from "../ipc/bindings";

/** Gutter and tree glyph of a status (`null`: not run yet). */
export function statusGlyph(status: TestStatus | null | undefined): string {
  switch (status) {
    case "passed":
      return "✓";
    case "failed":
      return "✗";
    case "skipped":
      return "○";
    case "running":
      return "◌";
    default:
      return "▶";
  }
}

/** "12 ms", "1.5 s", "2 min 5 s". */
export function formatDuration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return "";
  if (ms < 1000) return `${String(Math.round(ms))} ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)} s`;
  const minutes = Math.floor(seconds / 60);
  return `${String(minutes)} min ${String(Math.round(seconds - minutes * 60))} s`;
}

/** "Exit code 1 · 1.5 s", "Stopped · Exit code 130 · 3.0 s", "Killed · 2 ms". */
export function exitText(
  code: number | null | undefined,
  durationMs: number | null,
  stopped = false,
): string {
  const parts: string[] = [];
  if (stopped) parts.push("Stopped");
  if (code !== null && code !== undefined) parts.push(`Exit code ${String(code)}`);
  else if (!stopped) parts.push("Killed");
  const duration = formatDuration(durationMs);
  if (duration) parts.push(duration);
  return parts.join(" · ");
}

/** "1 failed, 3 passed, 1 skipped of 5 tests". */
export function summaryText(counts: readonly [number, number, number]): string {
  const [passed, failed, skipped] = counts;
  const total = passed + failed + skipped;
  const parts: string[] = [];
  if (failed > 0) parts.push(`${String(failed)} failed`);
  parts.push(`${String(passed)} passed`);
  if (skipped > 0) parts.push(`${String(skipped)} skipped`);
  return `${parts.join(", ")} of ${String(total)} ${total === 1 ? "test" : "tests"}`;
}

/** A visible row of the test tree. */
export interface TestRow {
  node: TestTreeNode;
  depth: number;
  /** Stable key: file path and titles. */
  key: string;
  hasChildren: boolean;
  collapsed: boolean;
}

export function nodeKey(node: TestTreeNode): string {
  return [node.location.path, ...node.fullName].join("\u0000");
}

function matches(node: TestTreeNode, shown: ReadonlySet<TestStatus>): boolean {
  if (node.kind === "test") return shown.has(node.status);
  return node.children.some((c) => matches(c, shown));
}

/**
 * Rows of the tree with only tests whose status is in `shown` (and their
 * suites and files), skipping the children of collapsed nodes.
 */
export function visibleRows(
  nodes: readonly TestTreeNode[],
  shown: ReadonlySet<TestStatus>,
  collapsed: ReadonlySet<string>,
): TestRow[] {
  const rows: TestRow[] = [];
  const walk = (list: readonly TestTreeNode[], depth: number) => {
    for (const node of list) {
      if (!matches(node, shown)) continue;
      const key = nodeKey(node);
      const isCollapsed = collapsed.has(key);
      rows.push({
        node,
        depth,
        key,
        hasChildren: node.children.length > 0,
        collapsed: isCollapsed,
      });
      if (!isCollapsed) walk(node.children, depth + 1);
    }
  };
  walk(nodes, 0);
  return rows;
}
