import { describe, expect, it } from "vitest";

import { alignGuides } from "../guides";

const vb = { minX: 0, minY: 0, width: 100, height: 100 };

describe("alignGuides", () => {
  it("snaps a moving bbox's edges to another shape's edges", () => {
    const moving = { minX: 12, minY: 40, maxX: 32, maxY: 60 };
    const other = { minX: 10, minY: 0, maxX: 30, maxY: 20 };
    const g = alignGuides(moving, [other], vb, 5);
    expect(g.dx).toBeCloseTo(-2); // left 12 → 10
    expect(g.gx).toContain(10);
  });

  it("aligns the centre to the canvas centre", () => {
    const moving = { minX: 40, minY: 40, maxX: 62, maxY: 60 }; // centreX 51
    const g = alignGuides(moving, [], vb, 3);
    expect(g.dx).toBeCloseTo(-1); // 51 → 50
    expect(g.gx).toContain(50);
  });

  it("returns no offset when nothing is within threshold", () => {
    const moving = { minX: 500, minY: 500, maxX: 520, maxY: 520 };
    const g = alignGuides(moving, [], vb, 3);
    expect(g.dx).toBe(0);
    expect(g.dy).toBe(0);
    expect(g.gx).toHaveLength(0);
  });
});
