import { beforeEach, describe, expect, it } from "vitest";

import { renderFatal, renderedAppName } from "./shell";

describe("shell", () => {
  let root: HTMLElement;

  beforeEach(() => {
    document.body.innerHTML = '<div id="app"><span data-testid="app-name">Ferrule</span></div>';
    const element = document.getElementById("app");
    if (!element) throw new Error("test setup failed: #app missing");
    root = element;
  });

  it("reads the rendered product name", () => {
    expect(renderedAppName(root)).toBe("Ferrule");
  });

  it("renders fatal errors as text, not HTML", () => {
    renderFatal(root, "<b>boom</b>");
    const alert = root.querySelector("[role='alert']");
    expect(alert?.textContent).toBe("<b>boom</b>");
    expect(root.querySelector("b")).toBeNull();
    expect(renderedAppName(root)).toBe("");
  });
});
