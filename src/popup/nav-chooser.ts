// Chooser for Go to Declaration / Type Declaration / Implementation when
// the language server returns several targets.
import type { NavTarget } from "../ipc/bindings";
import { VirtualList } from "../ui/virtual-list";
import { bindListKeys, Popup } from "./popup";

export class NavChooser extends Popup {
  private readonly list: VirtualList<NavTarget>;

  constructor(
    title: string,
    private readonly targets: readonly NavTarget[],
    choose: (target: NavTarget) => void,
  ) {
    super("nav-chooser", "nav-chooser", title);
    const heading = document.createElement("div");
    heading.className = "popup-title";
    heading.textContent = title;
    this.list = new VirtualList<NavTarget>(
      26,
      (target) => {
        const row = document.createElement("div");
        row.dataset.testid = "nav-target";
        row.dataset.path = target.path;
        const name = document.createElement("span");
        name.className = "se-name";
        name.textContent = `${target.path.split(/[\\/]/).pop() ?? target.path}:${String(target.line + 1)}`;
        const preview = document.createElement("span");
        preview.className = "se-detail nav-preview";
        preview.textContent = target.preview;
        const dir = document.createElement("span");
        dir.className = "se-detail";
        dir.textContent = target.path;
        row.append(name, preview, dir);
        return row;
      },
      (target) => {
        this.close();
        choose(target);
      },
      "nav-targets",
    );
    this.element.tabIndex = -1;
    this.element.append(heading, this.list.element);
    bindListKeys(this.element, this.list);
  }

  override open(): void {
    super.open();
    // Render once attached, so the list knows its height.
    this.list.setItems(this.targets);
    this.element.focus();
  }
}
