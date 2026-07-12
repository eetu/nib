import { describe, expect, it } from "vitest";

import { cubicAt } from "../geometry";
import {
  closeSubpath,
  insertNodeAt,
  nearestOnSubpath,
  parsePathD,
  pathToD,
  segmentControlPoints,
} from "../path";

describe("closeSubpath", () => {
  it("closes an open subpath and emits a trailing Z", () => {
    const sp = parsePathD("M 0 0 L 10 0 L 10 10")[0];
    closeSubpath(sp);
    expect(sp.closed).toBe(true);
    expect(pathToD([sp])).toBe("M 0 0 L 10 0 L 10 10 Z");
  });

  it("folds a coincident endpoint instead of leaving a zero-length seam", () => {
    // last node already sits on the start
    const sp = parsePathD("M 0 0 L 10 0 L 0 0")[0];
    expect(sp.nodes).toHaveLength(3);
    closeSubpath(sp);
    expect(sp.nodes).toHaveLength(2);
    expect(sp.closed).toBe(true);
  });
});

describe("insertNodeAt", () => {
  it("inserts a midpoint on a straight segment without changing the line", () => {
    const sp = parsePathD("M 0 0 L 10 0")[0];
    const idx = insertNodeAt(sp, 0, 0.5);
    expect(idx).toBe(1);
    expect(sp.nodes).toHaveLength(3);
    expect(sp.nodes[1].point).toEqual({ x: 5, y: 0 });
    expect(sp.nodes[1].handleIn).toBeUndefined();
  });

  it("splits a cubic while preserving its shape", () => {
    const sp = parsePathD("M 0 0 C 0 10 10 10 10 0")[0];
    // sample the original curve midpoint
    const [p0, p1, p2, p3] = segmentControlPoints(sp, 0);
    const mid = cubicAt(p0, p1, p2, p3, 0.5);
    insertNodeAt(sp, 0, 0.5);
    expect(sp.nodes).toHaveLength(3);
    expect(sp.nodes[1].point.x).toBeCloseTo(mid.x, 6);
    expect(sp.nodes[1].point.y).toBeCloseTo(mid.y, 6);
    expect(sp.nodes[1].handleIn).toBeDefined();
    expect(sp.nodes[1].handleOut).toBeDefined();
  });
});

describe("nearestOnSubpath", () => {
  it("finds the closest segment + parameter to a probe point", () => {
    const sp = parsePathD("M 0 0 L 10 0 L 10 10")[0];
    const hit = nearestOnSubpath(sp, { x: 5, y: 1 });
    expect(hit?.segmentIndex).toBe(0);
    expect(hit?.point.y).toBeCloseTo(0, 6);
  });

  it("considers the closing segment of a closed subpath", () => {
    const sp = parsePathD("M 0 0 L 10 0 L 10 10 Z")[0];
    // a point near the closing edge from (10,10) back to (0,0)
    const hit = nearestOnSubpath(sp, { x: 5, y: 5 });
    expect(hit?.segmentIndex).toBe(2);
  });
});
