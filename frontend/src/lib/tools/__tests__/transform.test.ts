import { describe, expect, it } from "vitest";

import {
  boundsCorners,
  boxFrame,
  frameHit,
  insideQuad,
  ROTATE_KNOB_PX,
  transformCursor,
} from "../transform";

const bb = { minX: 0, minY: 0, maxX: 100, maxY: 40 };

describe("boundsCorners", () => {
  it("walks nw → ne → se → sw", () => {
    expect(boundsCorners(bb)).toEqual([
      { x: 0, y: 0 },
      { x: 100, y: 0 },
      { x: 100, y: 40 },
      { x: 0, y: 40 },
    ]);
  });

  it("turns the whole box about the pivot, keeping its edge lengths", () => {
    const spun = boundsCorners(bb, { pivot: { x: 50, y: 20 }, angle: Math.PI / 2 });
    const [nw, ne, se] = spun;
    // A quarter turn: the 100-long top edge now runs vertically, the 40 side horizontally.
    expect(Math.hypot(ne.x - nw.x, ne.y - nw.y)).toBeCloseTo(100);
    expect(Math.hypot(se.x - ne.x, se.y - ne.y)).toBeCloseTo(40);
    expect(ne.x - nw.x).toBeCloseTo(0);
  });
});

describe("boxFrame", () => {
  it("puts the eight handles on the box, in the order the overlay draws them", () => {
    const f = boxFrame(...boundsCorners(bb));
    expect(f.handles.map((h) => h.handle)).toEqual(["nw", "n", "ne", "e", "se", "s", "sw", "w"]);
    expect(f.handles.find((h) => h.handle === "e")?.point).toEqual({ x: 100, y: 20 });
    expect(f.angle).toBeCloseTo(0);
  });

  it("hangs the knob off the top edge, outside the box", () => {
    const f = boxFrame(...boundsCorners(bb));
    expect(f.stem).toEqual({ x: 50, y: 0 });
    expect(f.knob).toEqual({ x: 50, y: -ROTATE_KNOB_PX });
  });

  it("keeps the knob outside a turned box, and reports the tilt", () => {
    const turned = boundsCorners(bb, { pivot: { x: 50, y: 20 }, angle: Math.PI });
    const f = boxFrame(...turned);
    // Upside down: the knob hangs *below* what is now the top edge.
    expect(f.knob.y).toBeCloseTo(40 + ROTATE_KNOB_PX);
    expect(Math.abs(f.angle)).toBeCloseTo(Math.PI);
  });

  it("keeps the knob outside a box with no height", () => {
    const flat = { minX: 0, minY: 10, maxX: 100, maxY: 10 };
    const f = boxFrame(...boundsCorners(flat));
    expect(f.knob.y).toBeCloseTo(10 - ROTATE_KNOB_PX);
  });
});

describe("frameHit", () => {
  const f = boxFrame(...boundsCorners(bb));

  it("finds the knob and each handle", () => {
    expect(frameHit(f, f.knob)).toEqual({ t: "rotate" });
    expect(frameHit(f, { x: 100, y: 40 })).toEqual({ t: "scale", handle: "se" });
  });

  it("misses the middle of the box", () => {
    expect(frameHit(f, { x: 50, y: 20 })).toBeNull();
  });
});

describe("insideQuad", () => {
  const upright = boundsCorners(bb);
  // A 100×40 box turned 45° about its centre: its corners reach well beyond its own edges.
  const turned = boundsCorners(bb, { pivot: { x: 50, y: 20 }, angle: Math.PI / 4 });

  it("accepts the interior and rejects the outside", () => {
    expect(insideQuad(upright, { x: 50, y: 20 })).toBe(true);
    expect(insideQuad(upright, { x: 150, y: 20 })).toBe(false);
  });

  it("accepts a point just outside, within the pad", () => {
    expect(insideQuad(upright, { x: -2, y: 20 })).toBe(false);
    expect(insideQuad(upright, { x: -2, y: 20 }, 3)).toBe(true);
  });

  it("follows a turned box rather than its axis-aligned bounds", () => {
    // The centre is inside either way; this corner of the *bounds* is outside the turned box.
    expect(insideQuad(turned, { x: 50, y: 20 })).toBe(true);
    const bounds = {
      minX: Math.min(...turned.map((p) => p.x)),
      minY: Math.min(...turned.map((p) => p.y)),
    };
    expect(insideQuad(turned, { x: bounds.minX + 1, y: bounds.minY + 1 })).toBe(false);
  });
});

describe("transformCursor", () => {
  it("names the axis each handle pulls along on an upright box", () => {
    expect(transformCursor("e")).toBe("ew-resize");
    expect(transformCursor("n")).toBe("ns-resize");
    expect(transformCursor("se")).toBe("nwse-resize");
    expect(transformCursor("ne")).toBe("nesw-resize");
  });

  it("follows the box's tilt", () => {
    // Turned a quarter, the east handle pulls up-down.
    expect(transformCursor("e", Math.PI / 2)).toBe("ns-resize");
    // ...and an eighth turn puts it on the diagonal.
    expect(transformCursor("e", Math.PI / 4)).toBe("nwse-resize");
  });

  it("is unchanged by a half turn — a resize axis has no direction", () => {
    for (const h of ["n", "ne", "e", "se"] as const)
      expect(transformCursor(h, Math.PI)).toBe(transformCursor(h));
  });
});
