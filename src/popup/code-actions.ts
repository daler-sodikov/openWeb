import type { CodeActionView } from "../ipc/bindings";
import { VirtualList } from "../ui/virtual-list";
import { bindListKeys, Popup } from "./popup";

export class CodeActionsPopup extends Popup {
  private readonly list: VirtualList<CodeActionView>;
  private readonly actions: readonly CodeActionView[];

  constructor(actions: readonly CodeActionView[], apply: (id: number) => void) {
    super("code-actions", "code-actions", "Intentions and Quick Fixes");
    this.actions = actions;
    const title = document.createElement("div");
    title.className = "popup-title";
    title.textContent = "Intentions and Quick Fixes";
    this.list = new VirtualList<CodeActionView>(
      30,
      (action) => {
        const row = document.createElement("div");
        row.dataset.testid = "code-action";
        const name = document.createElement("span");
        name.className = "se-name";
        name.textContent = action.title;
        row.append(name);
        if (action.kind) {
          const kind = document.createElement("span");
          kind.className = "se-detail";
          kind.textContent = action.kind;
          row.append(kind);
        }
        if (action.preferred) row.dataset.preferred = "true";
        if (action.disabled) {
          row.classList.add("disabled");
          row.title = action.disabled;
        }
        return row;
      },
      (action) => {
        if (action.disabled) return;
        this.close();
        apply(action.id);
      },
      "code-action-list",
    );
    this.element.tabIndex = -1;
    this.element.append(title, this.list.element);
    bindListKeys(this.element, this.list);
    if (actions.length === 0) {
      const empty = document.createElement("div");
      empty.className = "popup-empty";
      empty.textContent = "No fixes or refactorings are available here.";
      this.element.append(empty);
    }
  }

  override open(): void {
    super.open();
    this.list.setItems(this.actions);
    this.element.focus();
  }
}
