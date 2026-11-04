import { describe, expect, it } from "vitest";

import type { DiagnosticSpan, Severity } from "../ipc/bindings";
import { buildDiagnostics, diagnosticsAt, insightDom } from "./insight";

const span = (
  from: number,
  to: number,
  severity: Severity,
  message: string = severity,
): DiagnosticSpan => ({
  from,
  to,
  severity,
  message,
  source: "typescript",
  code: "2304",
  unnecessary: false,
  deprecated: false,
});

describe("buildDiagnostics", () => {
  it("widens empty ranges, clamps to the document and finds the most severe first", () => {
    const set = buildDiagnostics(
      [span(4, 4, "warning"), span(2, 6, "error"), span(8, 99, "hint"), span(50, 60, "error")],
      10,
    );
    const ranges: [number, number][] = [];
    set.between(0, 10, (from, to) => {
      ranges.push([from, to]);
    });
    // Overlapping ranges are not visited in start order; compare sorted.
    expect(ranges.sort((a, b) => a[0] - b[0])).toEqual([
      [2, 6],
      [4, 5],
      [8, 10],
    ]);
    expect(diagnosticsAt(set, 4).map((d) => d.severity)).toEqual(["error", "warning"]);
    expect(diagnosticsAt(set, 7)).toEqual([]);
  });

  it("keeps an empty range at the very end visible", () => {
    const set = buildDiagnostics([span(3, 3, "error")], 3);
    const ranges: [number, number][] = [];
    set.between(0, 3, (from, to) => {
      ranges.push([from, to]);
    });
    expect(ranges).toEqual([[2, 3]]);
  });
});

describe("insightDom", () => {
  it("shows messages as text and documentation from Rust, without navigating links", () => {
    const dom = insightDom(
      [span(0, 1, "error", "<b>not html</b>")],
      '<p>Docs <a href="https://example.com">link</a></p>',
    );
    const problem = dom.querySelector("[data-testid=insight-problem]");
    expect(problem?.textContent).toContain("<b>not html</b>");
    expect(problem?.querySelector("b")).toBeNull();
    expect(problem?.textContent).toContain("typescript 2304");
    const link = dom.querySelector("a");
    expect(link?.title).toBe("https://example.com/");
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    link?.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
  });
});
