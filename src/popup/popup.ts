// Modal popup base: overlay, Escape and outside clicks close it.

export class Popup {
  readonly element: HTMLElement;
  private readonly overlay: HTMLElement;
  private closed = false;
  onClose: (() => void) | null = null;

  constructor(className: string, testid: string, label: string) {
    this.overlay = document.createElement("div");
    this.overlay.className = "popup-overlay";
    this.element = document.createElement("div");
    this.element.className = `popup ${className}`;
    this.element.dataset.testid = testid;
    this.element.setAttribute("role", "dialog");
    this.element.setAttribute("aria-label", label);
    this.overlay.append(this.element);
    this.overlay.addEventListener("mousedown", (event) => {
      if (event.target === this.overlay) this.close();
    });
    this.element.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        this.close();
      }
    });
  }

  open(): void {
    document.body.append(this.overlay);
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.overlay.remove();
    this.onClose?.();
  }

  get isClosed(): boolean {
    return this.closed;
  }
}

export function input(className: string, testid: string, placeholder: string): HTMLInputElement {
  const el = document.createElement("input");
  el.type = "text";
  el.className = className;
  el.dataset.testid = testid;
  el.placeholder = placeholder;
  el.spellcheck = false;
  el.autocomplete = "off";
  return el;
}

/** Arrow keys and Enter drive a list from a text input. */
export function bindListKeys(
  field: HTMLElement,
  list: { move(delta: number): void; activateSelected(): void },
): void {
  field.addEventListener("keydown", (event) => {
    switch (event.key) {
      case "ArrowDown":
        list.move(1);
        break;
      case "ArrowUp":
        list.move(-1);
        break;
      case "PageDown":
        list.move(10);
        break;
      case "PageUp":
        list.move(-10);
        break;
      case "Enter":
        list.activateSelected();
        break;
      default:
        return;
    }
    event.preventDefault();
  });
}
