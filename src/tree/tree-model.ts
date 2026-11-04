import type { DirEntry } from "../ipc/bindings";

/** One visible row of the project tree. */
export interface TreeRow {
  entry: DirEntry;
  depth: number;
  expanded: boolean;
}

/**
 * Flattens the loaded part of the tree into visible rows. Children of an
 * expanded directory that have not been loaded yet are simply absent.
 */
export function flattenTree(
  root: readonly DirEntry[],
  children: ReadonlyMap<string, readonly DirEntry[]>,
  expanded: ReadonlySet<string>,
): TreeRow[] {
  const rows: TreeRow[] = [];
  const visit = (entries: readonly DirEntry[], depth: number) => {
    for (const entry of entries) {
      const isOpen = entry.kind === "directory" && expanded.has(entry.path);
      rows.push({ entry, depth, expanded: isOpen });
      const kids = isOpen ? children.get(entry.path) : undefined;
      if (kids) visit(kids, depth + 1);
    }
  };
  visit(root, 0);
  return rows;
}

/** Index range of rows to render for a scroll position (with overscan). */
export function visibleRange(
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  rowCount: number,
  overscan = 8,
): { start: number; end: number } {
  const start = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const end = Math.min(rowCount, Math.ceil((scrollTop + viewportHeight) / rowHeight) + overscan);
  return { start, end: Math.max(start, end) };
}
