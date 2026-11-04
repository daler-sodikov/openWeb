/** Replaces the window content with an error message that cannot be dismissed. */
export function renderFatal(root: HTMLElement, message: string): void {
  const error = document.createElement("p");
  error.className = "shell-fatal";
  error.setAttribute("role", "alert");
  error.textContent = message;
  root.replaceChildren(error);
}

/** Returns the product name currently rendered in the toolbar, if any. */
export function renderedAppName(root: HTMLElement): string {
  return root.querySelector<HTMLElement>("[data-testid='app-name']")?.textContent ?? "";
}
