// Virtualized list with keyboard selection: only visible rows are in the DOM.
import { visibleRange } from "../tree/tree-model";

export class VirtualList<T> {
  readonly element: HTMLElement;
  private readonly spacer: HTMLElement;
  private items: readonly T[] = [];
  private selected = 0;
  private frame = 0;

  constructor(
    private readonly rowHeight: number,
    private readonly renderRow: (item: T, selected: boolean) => HTMLElement,
    private readonly onActivate: (item: T) => void,
    testid: string,
  ) {
    this.element = document.createElement("div");
    this.element.className = "vlist";
    this.element.dataset.testid = testid;
    this.element.setAttribute("role", "listbox");
    this.spacer = document.createElement("div");
    this.spacer.className = "vlist-spacer";
    this.element.append(this.spacer);
    this.element.addEventListener("scroll", () => {
      this.requestRender();
    });
  }

  setItems(items: readonly T[]): void {
    this.items = items;
    this.selected = 0;
    this.element.scrollTop = 0;
    this.spacer.style.height = `${String(items.length * this.rowHeight)}px`;
    this.render();
  }

  get length(): number {
    return this.items.length;
  }

  selectedItem(): T | undefined {
    return this.items[this.selected];
  }

  move(delta: number): void {
    if (this.items.length === 0) return;
    this.selected = Math.max(0, Math.min(this.items.length - 1, this.selected + delta));
    const top = this.selected * this.rowHeight;
    if (top < this.element.scrollTop) this.element.scrollTop = top;
    else if (top + this.rowHeight > this.element.scrollTop + this.element.clientHeight)
      this.element.scrollTop = top + this.rowHeight - this.element.clientHeight;
    this.render();
  }

  activateSelected(): void {
    const item = this.selectedItem();
    if (item !== undefined) this.onActivate(item);
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
      this.element.clientHeight || 400,
      this.rowHeight,
      this.items.length,
    );
    const rows: HTMLElement[] = [];
    for (let i = start; i < end; i++) {
      const item = this.items[i];
      if (item === undefined) continue;
      const row = this.renderRow(item, i === this.selected);
      row.classList.add("vlist-row");
      if (i === this.selected) row.classList.add("selected");
      row.setAttribute("role", "option");
      row.setAttribute("aria-selected", String(i === this.selected));
      row.style.top = `${String(i * this.rowHeight)}px`;
      row.style.height = `${String(this.rowHeight)}px`;
      row.addEventListener("mousedown", (event) => {
        event.preventDefault();
        this.selected = i;
        this.onActivate(item);
      });
      rows.push(row);
    }
    this.spacer.replaceChildren(...rows);
  }
}

/** Renders `text` with the chars at `positions` (char indices) marked. */
export function highlighted(
  text: string,
  positions: readonly number[],
  className = "",
): HTMLElement {
  const span = document.createElement("span");
  if (className) span.className = className;
  const marks = new Set(positions);
  let buffer = "";
  let index = 0;
  const flush = () => {
    if (buffer) span.append(buffer);
    buffer = "";
  };
  for (const ch of text) {
    if (marks.has(index)) {
      flush();
      const mark = document.createElement("mark");
      mark.textContent = ch;
      span.append(mark);
    } else {
      buffer += ch;
    }
    index++;
  }
  flush();
  return span;
}
