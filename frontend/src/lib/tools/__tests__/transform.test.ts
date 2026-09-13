import { describe, expect, it } from "vitest";

import type { Subpath } from "$lib/model/types";

import {
  boundsCorners,
  boxFrame,
  distortSubpaths,
  framedCenter,
  framedCorners,
  frameHit,
  fromFrame,
  insideQuad,
  orientedBounds,
  ROTATE_KNOB_PX,
  rotateSubpaths,
  scaleSubpathsFramed,
  toFrame,
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

// --- a shape's own frame -----------------------------------------------------------------------
// A rotation bakes into a path's anchors, so `boxAngle` is the only record of which way it was
// turned. These are the round trips that turn that one number into a box that hugs the shape and
// handles that pull along its edges.

const square: Subpath[] = [
  {
    closed: true,
    nodes: [
      { type: "corner", point: { x: 10, y: 0 } },
      { type: "corner", point: { x: 30, y: 0 } },
      { type: "corner", point: { x: 30, y: 10 } },
      { type: "corner", point: { x: 10, y: 10 } },
    ],
  },
];

describe("toFrame / fromFrame", () => {
  it("round-trip to the same point", () => {
    const p = { x: 7, y: -3 };
    const back = fromFrame(toFrame(p, 0.7), 0.7);
    expect(back.x).toBeCloseTo(p.x);
    expect(back.y).toBeCloseTo(p.y);
  });

  it("are identity at no tilt", () => {
    expect(toFrame({ x: 5, y: 9 }, 0)).toEqual({ x: 5, y: 9 });
  });
});

describe("orientedBounds", () => {
  it("is the plain tight box at no tilt", () => {
    expect(orientedBounds(square, 0)).toEqual({ minX: 10, minY: 0, maxX: 30, maxY: 10 });
  });

  it("keeps a turned shape's own size, where axis-aligned bounds would not", () => {
    const angle = Math.PI / 6;
    const turned = rotateSubpaths(square, { x: 20, y: 5 }, angle);
    const own = orientedBounds(turned, angle)!;
    expect(own.maxX - own.minX).toBeCloseTo(20);
    expect(own.maxY - own.minY).toBeCloseTo(10);
    // The document-axis box of the same geometry is bigger in both directions — which is exactly
    // why a box drawn from it can't hug the shape.
    const axis = orientedBounds(turned, 0)!;
    expect(axis.maxX - axis.minX).toBeGreaterThan(20.5);
    expect(axis.maxY - axis.minY).toBeGreaterThan(10.5);
  });
});

describe("framedCorners / framedCenter", () => {
  it("put the box back where the shape is", () => {
    const angle = Math.PI / 6;
    const turned = rotateSubpaths(square, { x: 20, y: 5 }, angle);
    const own = orientedBounds(turned, angle)!;
    const corners = framedCorners(own, angle);
    // Each corner of the shape's own box coincides with a corner of the turned square.
    for (const node of turned[0].nodes) {
      const near = Math.min(
        ...corners.map((c) => Math.hypot(c.x - node.point.x, c.y - node.point.y)),
      );
      expect(near).toBeCloseTo(0);
    }
    // And the centre is the shape's centre — not the centre of its document-axis bounds.
    const c = framedCenter(own, angle);
    expect(c.x).toBeCloseTo(20);
    expect(c.y).toBeCloseTo(5);
  });
});

describe("scaleSubpathsFramed", () => {
  it("scales along a tilted box's own axes, holding the anchor still", () => {
    const angle = Math.PI / 4;
    const turned = rotateSubpaths(square, { x: 20, y: 5 }, angle);
    const own = orientedBounds(turned, angle)!;
    const anchor = framedCorners(own, angle)[0]; // nw — the fixed corner when dragging se
    const out = scaleSubpathsFramed(turned, anchor, 2, 1, angle);

    // The anchor stayed put...
    const stuck = Math.min(
      ...out[0].nodes.map((n) => Math.hypot(n.point.x - anchor.x, n.point.y - anchor.y)),
    );
    expect(stuck).toBeCloseTo(0);
    // ...and the shape is twice as wide *in its own frame*, exactly as tall.
    const after = orientedBounds(out, angle)!;
    expect(after.maxX - after.minX).toBeCloseTo(40);
    expect(after.maxY - after.minY).toBeCloseTo(10);
  });

  it("matches the plain scale at no tilt", () => {
    const a = { x: 10, y: 0 };
    const framed = scaleSubpathsFramed(square, a, 2, 3, 0);
    expect(orientedBounds(framed, 0)).toEqual({ minX: 10, minY: 0, maxX: 50, maxY: 30 });
  });
});

describe("distortSubpaths", () => {
  /** A unit square as one closed subpath, with a handle hung off the first node. */
  const square = (): Subpath[] => [
    {
      closed: true,
      nodes: [
        { type: "corner", point: { x: 0, y: 0 }, handleOut: { x: 0, y: 10 } },
        { type: "corner", point: { x: 10, y: 0 } },
        { type: "corner", point: { x: 10, y: 10 } },
        { type: "corner", point: { x: 0, y: 10 } },
      ],
    },
  ];

  it("pins the opposite edge and slides the dragged one, handles included", () => {
    // skewX about the bottom edge (y=10): x' = x + kx·(y - 10).
    const out = distortSubpaths(square(), 0, { x: 5, y: 10 }, [1, 0, 0.5, 1, 0, 0]);
    const n = out[0].nodes;
    // The bottom edge is the pivot line — it does not move.
    expect(n[2].point.x).toBeCloseTo(10);
    expect(n[3].point.x).toBeCloseTo(0);
    // The top edge slides left by kx·height (y - 10 = -10 there).
    expect(n[0].point.x).toBeCloseTo(-5);
    expect(n[1].point.x).toBeCloseTo(5);
    // Heights are untouched by a pure skewX.
    expect(n[0].point.y).toBeCloseTo(0);
    // The handle rides along. Left behind, it would silently reshape the curve — the whole
    // reason an affine transform maps control points rather than re-fitting the outline.
    expect(n[0].handleOut?.x).toBeCloseTo(0);
    expect(n[0].handleOut?.y).toBeCloseTo(10);
  });

  it("shears along the shape's OWN axes when the box is turned", () => {
    // The point of going through the frame. A skewX on a box turned 30° must slide the shape
    // along THAT box's top edge; done in document space it would slide along the document's x and
    // the shape would walk out from under its own box.
    const angle = Math.PI / 6;
    const ref = square();
    const out = distortSubpaths(ref, angle, { x: 5, y: 10 }, [1, 0, 0.5, 1, 0, 0]);
    // Every point moves parallel to the frame's x-axis — the direction the box's top edge runs.
    const axis = { x: Math.cos(angle), y: Math.sin(angle) };
    out[0].nodes.forEach((n, i) => {
      const d = { x: n.point.x - ref[0].nodes[i].point.x, y: n.point.y - ref[0].nodes[i].point.y };
      const len = Math.hypot(d.x, d.y);
      if (len < 1e-9) return; // a point on the pivot line doesn't move at all
      // Collinear with the axis, not merely in its direction: points on opposite sides of the
      // pivot line slide opposite ways, which is what makes it a shear rather than a translation.
      expect(Math.abs((d.x * axis.y - d.y * axis.x) / len)).toBeLessThan(1e-9);
    });
    // ...and at least one point actually moved, so the assertion above isn't vacuous.
    expect(
      out[0].nodes.some(
        (n, i) =>
          Math.hypot(n.point.x - ref[0].nodes[i].point.x, n.point.y - ref[0].nodes[i].point.y) >
          1e-6,
      ),
    ).toBe(true);
  });

  it("leaves the reference geometry untouched", () => {
    const ref = square();
    distortSubpaths(ref, 0, { x: 5, y: 10 }, [1, 0, 0.5, 1, 0, 0]);
    expect(ref[0].nodes[0].point).toEqual({ x: 0, y: 0 });
  });
});
