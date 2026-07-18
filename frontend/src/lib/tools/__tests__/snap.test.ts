import { describe, expect, it } from "vitest";

import { gridSnapBox } from "../guides";

describe("gridSnapBox", () => {
  it("snaps the nearest box edge to a grid line, per axis", () => {
    // A 10×10 box at (0.3, 0.3): its min/max both sit 0.3 past a grid line, the centre 4.7 away →
    // the nearest correction pulls min (and max) back onto the grid.
    const g = gridSnapBox({ minX: 0.3, minY: 0.3, maxX: 10.3, maxY: 10.3 }, 10);
    expect(g.dx).toBeCloseTo(-0.3);
    expect(g.dy).toBeCloseTo(-0.3);
  });

  it("can snap by the centre when that edge is closest to a grid line", () => {
    // A 4-wide box centred on x=10 (a grid line): centre is exactly on grid, edges are ±2 off →
    // the centre wins, so no correction.
    const g = gridSnapBox({ minX: 8, minY: 8, maxX: 12, maxY: 12 }, 10);
    expect(g.dx).toBeCloseTo(0);
    expect(g.dy).toBeCloseTo(0);
  });

  it("is a no-op for a non-positive grid size", () => {
    expect(gridSnapBox({ minX: 3, minY: 7, maxX: 9, maxY: 11 }, 0)).toEqual({ dx: 0, dy: 0 });
  });
});
