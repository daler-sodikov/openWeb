import { beforeEach, describe, expect, it } from "vitest";

import type { AppInfo } from "../ipc/bindings";
import { renderFatal, renderShell, renderedAppName } from "./shell";

const info: AppInfo = {
  name: "Ferrule",
  version: "1.2.3",
  protocolVersion: 1,
  platform: "linux",
};

describe("shell", () => {
  let root: HTMLElement;

  beforeEach(() => {
    document.body.innerHTML = '<div id="app"></div>';
    document.title = "";
    const element = document.getElementById("app");
    if (!element) throw new Error("test setup failed: #app missing");
    root = element;
  });

  it("renders product name, version and title from backend info", () => {
    renderShell(root, info);
    expect(renderedAppName(root)).toBe("Ferrule");
    expect(root.querySelector("[data-testid='app-version']")?.textContent).toBe("v1.2.3");
    expect(document.title).toBe("Ferrule");
  });

  it("replaces previous content on re-render", () => {
    renderShell(root, info);
    renderShell(root, { ...info, version: "2.0.0" });
    expect(root.querySelectorAll("[data-testid='app-name']")).toHaveLength(1);
    expect(root.querySelector("[data-testid='app-version']")?.textContent).toBe("v2.0.0");
  });

  it("renders fatal errors as text, not HTML", () => {
    renderFatal(root, "<b>boom</b>");
    const alert = root.querySelector("[role='alert']");
    expect(alert?.textContent).toBe("<b>boom</b>");
    expect(root.querySelector("b")).toBeNull();
    expect(renderedAppName(root)).toBe("");
  });
});
