// Search Everywhere (double Shift) with All / Files / Symbols / Actions tabs.
// Go to File, Go to Symbol and Find Action open it on their tab.
import { commands, type KeyAction, type SearchItem, type SearchTab } from "../ipc/bindings";
import { errorMessage, unwrap } from "../ipc/result";
import { highlighted, VirtualList } from "../ui/virtual-list";
import { bindListKeys, input, Popup } from "./popup";

export interface SearchEverywhereHost {
  openLocation(path: string, line?: number, column?: number): void;
  runAction(action: KeyAction): void;
  shortcutFor(action: KeyAction): string;
}

const TABS: { tab: SearchTab; label: string }[] = [
  { tab: "all", label: "All" },
  { tab: "files", label: "Files" },
  { tab: "symbols", label: "Symbols" },
  { tab: "actions", label: "Actions" },
];

const KIND_BADGES: Record<string, string> = {
  class: "C",
  interface: "I",
  function: "ƒ",
  method: "m",
  type: "T",
  enum: "E",
  module: "M",
  constant: "K",
};

export class SearchEverywhere extends Popup {
  private tab: SearchTab;
  private readonly field = input("popup-input", "se-input", "Search everywhere");
  private readonly tabBar = document.createElement("div");
  private readonly footer = document.createElement("div");
  private readonly list: VirtualList<SearchItem>;
  private seq = 0;

  constructor(
    private readonly host: SearchEverywhereHost,
    tab: SearchTab,
    initialQuery = "",
  ) {
    super("search-everywhere", "search-everywhere", "Search Everywhere");
    this.tab = tab;
    this.list = new VirtualList<SearchItem>(
      26,
      (item) => this.renderItem(item),
      (item) => {
        this.activate(item);
      },
      "se-results",
    );
    this.tabBar.className = "popup-tabs";
    this.footer.className = "popup-footer";
    this.footer.dataset.testid = "se-footer";
    this.field.value = initialQuery;
    this.element.append(this.tabBar, this.field, this.list.element, this.footer);
    bindListKeys(this.field, this.list);
    this.field.addEventListener("input", () => {
      void this.search();
    });
    this.field.addEventListener("keydown", (event) => {
      if (event.key === "Tab") {
        event.preventDefault();
        const index = TABS.findIndex((t) => t.tab === this.tab);
        const next = TABS[(index + (event.shiftKey ? TABS.length - 1 : 1)) % TABS.length];
        if (next) this.setTab(next.tab);
      }
    });
    this.renderTabs();
  }

  override open(): void {
    super.open();
    this.field.focus();
    this.field.select();
    void this.search();
  }

  setTab(tab: SearchTab): void {
    this.tab = tab;
    this.renderTabs();
    void this.search();
    this.field.focus();
  }

  private renderTabs(): void {
    this.tabBar.replaceChildren(
      ...TABS.map(({ tab, label }) => {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "popup-tab";
        button.textContent = label;
        button.dataset.testid = `se-tab-${tab}`;
        button.setAttribute("aria-pressed", String(tab === this.tab));
        button.addEventListener("mousedown", (event) => {
          event.preventDefault();
          this.setTab(tab);
        });
        return button;
      }),
    );
  }

  private async search(): Promise<void> {
    const seq = ++this.seq;
    try {
      const results = unwrap(await commands.searchEverywhere(this.field.value, this.tab));
      if (seq !== this.seq || this.isClosed) return;
      this.list.setItems(results.items);
      this.footer.textContent = results.indexing
        ? "Indexing… results may be incomplete"
        : results.items.length === 0
          ? "Nothing found"
          : "";
    } catch (error: unknown) {
      if (seq === this.seq) this.footer.textContent = errorMessage(error);
    }
  }

  private renderItem(item: SearchItem): HTMLElement {
    const row = document.createElement("div");
    row.className = `se-item se-${item.type}`;
    row.dataset.testid = "se-item";
    row.dataset.type = item.type;
    const kind = document.createElement("span");
    kind.className = "se-kind";
    const detail = document.createElement("span");
    detail.className = "se-detail";
    switch (item.type) {
      case "file": {
        const slash = item.path.lastIndexOf("/");
        const nameStart = Array.from(item.path.slice(0, slash + 1)).length;
        kind.textContent = "▤";
        row.dataset.path = item.path;
        row.append(
          kind,
          highlighted(
            item.path.slice(slash + 1),
            item.positions.filter((p) => p >= nameStart).map((p) => p - nameStart),
            "se-name",
          ),
        );
        detail.textContent = item.path.slice(0, Math.max(0, slash));
        break;
      }
      case "symbol":
        kind.textContent = KIND_BADGES[item.kind] ?? "•";
        kind.title = item.kind;
        row.dataset.path = item.path;
        row.append(kind, highlighted(item.name, item.positions, "se-name"));
        detail.textContent = `${item.path}:${String(item.line + 1)}`;
        break;
      case "action":
        kind.textContent = "▶";
        row.append(kind, highlighted(item.name, item.positions, "se-name"));
        detail.textContent = this.host.shortcutFor(item.action);
        break;
    }
    row.append(detail);
    return row;
  }

  private activate(item: SearchItem): void {
    this.close();
    switch (item.type) {
      case "file":
        this.host.openLocation(item.path);
        break;
      case "symbol":
        this.host.openLocation(item.path, item.line, item.column);
        break;
      case "action":
        this.host.runAction(item.action);
        break;
    }
  }
}
