import { describe, expect, it } from "vitest";

import { parsePathD, pathToD } from "../path";

describe("parsePathD", () => {
  it("parses a line-based open path into anchor nodes", () => {
    const sp = parsePathD("M 0 0 L 10 0 L 10 10");
    expect(sp).toHaveLength(1);
    expect(sp[0].closed).toBe(false);
    expect(sp[0].nodes.map((n) => n.point)).toEqual([
      { x: 0, y: 0 },
      { x: 10, y: 0 },
      { x: 10, y: 10 },
    ]);
    // straight segments carry no handles
    expect(sp[0].nodes.every((n) => !n.handleIn && !n.handleOut)).toBe(true);
  });

  it("captures cubic control handles on the adjoining nodes", () => {
    const sp = parsePathD("M 0 0 C 0 10 10 10 10 0");
    expect(sp[0].nodes).toHaveLength(2);
    expect(sp[0].nodes[0].handleOut).toEqual({ x: 0, y: 10 });
    expect(sp[0].nodes[1].handleIn).toEqual({ x: 10, y: 10 });
  });

  it("normalizes relative + shorthand commands to absolute cubics", () => {
    // h/v/relative should all resolve to absolute nodes
    const sp = parsePathD("M 5 5 h 10 v 10 z");
    expect(sp[0].closed).toBe(true);
    expect(sp[0].nodes.map((n) => n.point)).toEqual([
      { x: 5, y: 5 },
      { x: 15, y: 5 },
      { x: 15, y: 15 },
    ]);
  });

  it("folds a curve that closes exactly on the start point", () => {
    // last cubic ends on the start, then Z — the trailing node folds into node 0
    const sp = parsePathD("M 0 0 C 5 0 10 5 10 10 C 5 10 0 5 0 0 Z");
    expect(sp[0].closed).toBe(true);
    expect(sp[0].nodes).toHaveLength(2);
    expect(sp[0].nodes[0].handleIn).toEqual({ x: 0, y: 5 });
  });

  it("marks collinear-handle nodes as smooth", () => {
    // node 1 has handleIn (0,5) and handleOut (0,-5) — collinear through (5,5)
    const sp = parsePathD("M 0 5 C 0 5 5 10 5 5 C 5 0 10 5 10 5");
    expect(sp[0].nodes[1].type).toBe("smooth");
  });

  it("converts arcs to cubics (lossy) rather than dropping them", () => {
    const sp = parsePathD("M 0 0 A 5 5 0 0 1 10 0");
    // arc becomes one or more cubic segments — nodes beyond the start appear
    expect(sp[0].nodes.length).toBeGreaterThan(1);
    expect(sp[0].nodes.some((n) => n.handleIn || n.handleOut)).toBe(true);
  });
});

describe("pathToD round-trip", () => {
  it("round-trips an open line path", () => {
    const d = "M 0 0 L 10 0 L 10 10";
    expect(pathToD(parsePathD(d))).toBe("M 0 0 L 10 0 L 10 10");
  });

  it("round-trips a cubic and re-emits it as a curve", () => {
    const d = "M 0 0 C 0 10 10 10 10 0";
    expect(pathToD(parsePathD(d))).toBe("M 0 0 C 0 10 10 10 10 0");
  });

  it("emits Z for a straight-closed subpath", () => {
    const out = pathToD(parsePathD("M 0 0 L 10 0 L 10 10 Z"));
    expect(out).toBe("M 0 0 L 10 0 L 10 10 Z");
  });

  it("rounds coordinates to the requested precision", () => {
    const out = pathToD(parsePathD("M 0.123456 0 L 10 0"), 2);
    expect(out).toBe("M 0.12 0 L 10 0");
  });
});
