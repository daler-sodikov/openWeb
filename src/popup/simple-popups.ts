// Recent Files (Ctrl+E) and Go to Line (Ctrl+G).
import { commands } from "../ipc/bindings";
import { errorMessage, unwrap } from "../ipc/result";
import { VirtualList } from "../ui/virtual-list";
import { bindListKeys, input, Popup } from "./popup";

export class RecentFiles extends Popup {
  private readonly list: VirtualList<string>;
  private readonly message = document.createElement("div");

  constructor(private readonly openFile: (path: string) => void) {
    super("recent-files", "recent-files", "Recent Files");
    const title = document.createElement("div");
    title.className = "popup-title";
    title.textContent = "Recent Files";
    this.list = new VirtualList<string>(
      26,
      (path) => {
        const row = document.createElement("div");
        row.dataset.testid = "recent-item";
        row.dataset.path = path;
        const name = document.createElement("span");
        name.className = "se-name";
        name.textContent = path.split("/").pop() ?? path;
        const dir = document.createElement("span");
        dir.className = "se-detail";
        dir.textContent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
        row.append(name, dir);
        return row;
      },
      (path) => {
        this.close();
        this.openFile(path);
      },
      "recent-results",
    );
    this.message.className = "popup-footer";
    this.element.tabIndex = -1;
    this.element.append(title, this.list.element, this.message);
    bindListKeys(this.element, this.list);
  }

  override open(): void {
    super.open();
    this.element.focus();
    void (async () => {
      try {
        const recent = unwrap(await commands.recentFiles());
        this.list.setItems(recent);
        // Like IntelliJ: the current file is first, so preselect the previous one.
        if (recent.length > 1) this.list.move(1);
        this.message.textContent = recent.length === 0 ? "No recent files" : "";
      } catch (error: unknown) {
        this.message.textContent = errorMessage(error);
      }
    })();
  }
}

/** Parses "line[:column]" (1-based) into zero-based numbers. */
export function parseLineColumn(text: string): { line: number; column: number } | null {
  const match = /^\s*(\d+)\s*(?:[:,]\s*(\d+)\s*)?$/.exec(text);
  if (!match?.[1]) return null;
  const line = Number(match[1]) - 1;
  const column = match[2] ? Number(match[2]) - 1 : 0;
  return line < 0 || column < 0 ? null : { line, column };
}

export class GotoLine extends Popup {
  private readonly field: HTMLInputElement;
  private readonly hint = document.createElement("div");

  constructor(current: string, go: (line: number, column: number) => void) {
    super("goto-line", "goto-line", "Go to Line");
    const title = document.createElement("div");
    title.className = "popup-title";
    title.textContent = "Go to Line:Column";
    this.field = input("popup-input", "goto-line-input", current);
    this.field.value = current;
    this.hint.className = "popup-footer";
    this.element.append(title, this.field, this.hint);
    this.field.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      const target = parseLineColumn(this.field.value);
      if (!target) {
        this.hint.textContent = "Enter a line number, optionally followed by :column";
        return;
      }
      this.close();
      go(target.line, target.column);
    });
  }

  override open(): void {
    super.open();
    this.field.focus();
    this.field.select();
  }
}
