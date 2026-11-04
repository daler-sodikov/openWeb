import { describe, expect, it } from "vitest";

import { parseLineColumn } from "./simple-popups";

describe("parseLineColumn", () => {
  it("parses 1-based line and optional column", () => {
    expect(parseLineColumn("12")).toEqual({ line: 11, column: 0 });
    expect(parseLineColumn(" 3:7 ")).toEqual({ line: 2, column: 6 });
    expect(parseLineColumn("3,7")).toEqual({ line: 2, column: 6 });
  });

  it("rejects invalid input", () => {
    expect(parseLineColumn("")).toBeNull();
    expect(parseLineColumn("0")).toBeNull();
    expect(parseLineColumn("a:1")).toBeNull();
    expect(parseLineColumn("2:0")).toBeNull();
  });
});
