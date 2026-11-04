import type { AppInfo } from "../ipc/bindings";

/** Renders the application shell for the given backend info into `root`. */
export function renderShell(root: HTMLElement, info: AppInfo): void {
  document.title = info.name;

  const header = document.createElement("header");
  header.className = "shell-header";

  const name = document.createElement("h1");
  name.className = "shell-name";
  name.dataset.testid = "app-name";
  name.textContent = info.name;

  const version = document.createElement("span");
  version.className = "shell-version";
  version.dataset.testid = "app-version";
  version.textContent = `v${info.version}`;

  header.append(name, version);

  const status = document.createElement("p");
  status.className = "shell-status";
  status.dataset.testid = "app-status";
  status.textContent = "No project open.";

  const main = document.createElement("main");
  main.className = "shell-main";
  main.append(header, status);

  root.replaceChildren(main);
}

/** Replaces the shell with an error message that cannot be dismissed. */
export function renderFatal(root: HTMLElement, message: string): void {
  const error = document.createElement("p");
  error.className = "shell-fatal";
  error.setAttribute("role", "alert");
  error.textContent = message;
  root.replaceChildren(error);
}

/** Returns the product name currently rendered in the shell, if any. */
export function renderedAppName(root: HTMLElement): string {
  return root.querySelector<HTMLElement>("[data-testid='app-name']")?.textContent ?? "";
}
