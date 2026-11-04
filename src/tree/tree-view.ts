// Virtualized project tree. Only rows inside the viewport (plus overscan)
// exist in the DOM; directory contents are loaded lazily from Rust.
import type { DirEntry } from "../ipc/bindings";
import { flattenTree, type TreeRow, visibleRange } from "./tree-model";

export const TREE_ROW_HEIGHT = 22;

export interface TreeHost {
  listDir(path: string): Promise<DirEntry[]>;
  openFile(path: string): void;
  onError(message: string): void;
}

export class TreeView {
  private root: DirEntry[] = [];
  private readonly children = new Map<string, DirEntry[]>();
  private readonly expanded = new Set<string>();
  private rows: TreeRow[] = [];
  private selected: string | null = null;
  private readonly spacer: HTMLElement;
  private frame = 0;

  constructor(
    private readonly element: HTMLElement,
    private readonly host: TreeHost,
  ) {
    element.setAttribute("role", "tree");
    element.tabIndex = 0;
    this.spacer = document.createElement("div");
    this.spacer.className = "tree-spacer";
    element.replaceChildren(this.spacer);
    element.addEventListener("scroll", () => {
      this.requestRender();
    });
    element.addEventListener("keydown", (event) => {
      this.onKey(event);
    });
    new ResizeObserver(() => {
      this.requestRender();
    }).observe(element);
  }

  setRoot(entries: DirEntry[]): void {
    this.root = entries;
    this.children.clear();
    this.expanded.clear();
    this.selected = null;
    this.element.scrollTop = 0;
    this.refresh();
  }

  async toggle(path: string): Promise<void> {
    if (this.expanded.has(path)) {
      this.expanded.delete(path);
    } else {
      this.expanded.add(path);
      if (!this.children.has(path)) {
        try {
          this.children.set(path, await this.host.listDir(path));
        } catch (error: unknown) {
          this.expanded.delete(path);
          this.host.onError(error instanceof Error ? error.message : String(error));
        }
      }
    }
    this.refresh();
  }

  private refresh(): void {
    this.rows = flattenTree(this.root, this.children, this.expanded);
    this.spacer.style.height = `${String(this.rows.length * TREE_ROW_HEIGHT)}px`;
    this.render();
  }

  private requestRender(): void {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.render();
    });
  }

  private render(): void {
    const { start, end } = visibleRange(
      this.element.scrollTop,
      this.element.clientHeight || 600,
      TREE_ROW_HEIGHT,
      this.rows.length,
    );
    const items: HTMLElement[] = [];
    for (let i = start; i < end; i++) {
      const row = this.rows[i];
      if (row) items.push(this.renderRow(row, i));
    }
    this.spacer.replaceChildren(...items);
  }

  private renderRow(row: TreeRow, index: number): HTMLElement {
    const { entry } = row;
    const el = document.createElement("div");
    el.className = `tree-row tree-${entry.kind}`;
    if (entry.path === this.selected) el.classList.add("selected");
    el.setAttribute("role", "treeitem");
    el.setAttribute("aria-level", String(row.depth + 1));
    el.setAttribute("aria-selected", String(entry.path === this.selected));
    if (entry.kind === "directory") el.setAttribute("aria-expanded", String(row.expanded));
    el.dataset.testid = "tree-row";
    el.dataset.path = entry.path;
    el.style.top = `${String(index * TREE_ROW_HEIGHT)}px`;
    el.style.paddingLeft = `${String(8 + row.depth * 14)}px`;

    const twisty = document.createElement("span");
    twisty.className = "tree-twisty";
    if (entry.kind === "directory") twisty.classList.add(row.expanded ? "open" : "closed");
    const label = document.createElement("span");
    label.className = "tree-label";
    label.textContent = entry.name;
    el.append(twisty, label);

    el.addEventListener("click", () => {
      this.selected = entry.path;
      this.activate(entry);
    });
    return el;
  }

  private activate(entry: DirEntry): void {
    if (entry.kind === "directory") void this.toggle(entry.path);
    else {
      this.render();
      this.host.openFile(entry.path);
    }
  }

  private onKey(event: KeyboardEvent): void {
    const index = this.rows.findIndex((r) => r.entry.path === this.selected);
    const row = this.rows[index];
    const select = (i: number) => {
      const target = this.rows[Math.max(0, Math.min(i, this.rows.length - 1))];
      if (!target) return;
      this.selected = target.entry.path;
      const top = i * TREE_ROW_HEIGHT;
      if (top < this.element.scrollTop) this.element.scrollTop = top;
      else if (top + TREE_ROW_HEIGHT > this.element.scrollTop + this.element.clientHeight)
        this.element.scrollTop = top + TREE_ROW_HEIGHT - this.element.clientHeight;
      this.render();
    };
    switch (event.key) {
      case "ArrowDown":
        select(index + 1);
        break;
      case "ArrowUp":
        select(index <= 0 ? 0 : index - 1);
        break;
      case "ArrowRight":
        if (row?.entry.kind === "directory" && !row.expanded) void this.toggle(row.entry.path);
        break;
      case "ArrowLeft":
        if (row?.expanded) void this.toggle(row.entry.path);
        break;
      case "Enter":
        if (row) this.activate(row.entry);
        break;
      default:
        return;
    }
    event.preventDefault();
  }
}
