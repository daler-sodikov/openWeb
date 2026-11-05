import type { UsageResults } from "../ipc/bindings";
import { VirtualList } from "../ui/virtual-list";
import { bindListKeys, Popup } from "./popup";

interface UsageEntry {
  path: string;
  inProject: boolean;
  line: number;
  column: number;
  preview: string;
}

export class UsagesPopup extends Popup {
  private readonly list: VirtualList<UsageEntry>;
  private readonly entries: UsageEntry[];

  constructor(
    results: UsageResults,
    openLocation: (path: string, line: number, column: number) => void,
  ) {
    super("usages", "find-usages", `Find Usages: ${results.symbol}`);
    this.entries = results.groups.flatMap((group) =>
      group.usages.map((usage) => ({
        path: group.path,
        inProject: group.inProject,
        line: usage.line,
        column: usage.column,
        preview: usage.preview,
      })),
    );
    const title = document.createElement("div");
    title.className = "popup-title";
    title.textContent = `Find Usages: ${results.symbol}`;
    this.list = new VirtualList<UsageEntry>(
      42,
      (usage) => {
        const row = document.createElement("div");
        row.dataset.testid = "usage-result";
        const location = document.createElement("span");
        location.className = "se-name";
        location.textContent = `${usage.path}:${String(usage.line + 1)}`;
        if (!usage.inProject) location.classList.add("external");
        const preview = document.createElement("span");
        preview.className = "se-detail usage-preview";
        preview.textContent = usage.preview;
        row.append(location, preview);
        return row;
      },
      (usage) => {
        this.close();
        openLocation(usage.path, usage.line, usage.column);
      },
      "usage-results",
    );
    this.element.tabIndex = -1;
    this.element.append(title, this.list.element);
    bindListKeys(this.element, this.list);
    if (this.entries.length === 0) {
      const empty = document.createElement("div");
      empty.className = "popup-empty";
      empty.textContent = "No usages found.";
      this.element.append(empty);
    }
  }

  override open(): void {
    super.open();
    this.list.setItems(this.entries);
    this.element.focus();
  }
}
