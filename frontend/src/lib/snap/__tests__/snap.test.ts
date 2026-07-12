import { describe, expect, it } from "vitest";

import { parseSvg } from "$lib/model/document";
import type { NodeRef } from "$lib/model/types";

import { collectAnchors, findSnap, isCloseLoop, snapToGrid } from "../index";

function docWithOpenPath() {
  // an open triangle-ish path: start (0,0), (10,0), (10,10) — not closed
  return parseSvg(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path d="M 0 0 L 10 0 L 10 10"/></svg>`,
  );
}

describe("collectAnchors", () => {
  it("gathers every anchor and flags open-subpath endpoints", () => {
    const anchors = collectAnchors(docWithOpenPath());
    expect(anchors).toHaveLength(3);
    expect(anchors[0].endpoint).toBe(true); // first
    expect(anchors[1].endpoint).toBe(false); // middle
    expect(anchors[2].endpoint).toBe(true); // last
  });

  it("excludes the dragged node", () => {
    const exclude: NodeRef = { pathIndex: 0, subpathIndex: 0, nodeIndex: 2 };
    const anchors = collectAnchors(docWithOpenPath(), exclude);
    expect(anchors).toHaveLength(2);
    expect(anchors.some((a) => a.ref.nodeIndex === 2)).toBe(false);
  });
});

describe("findSnap", () => {
  it("returns the nearest candidate within threshold", () => {
    const anchors = collectAnchors(docWithOpenPath());
    const hit = findSnap({ x: 0.5, y: 0.5 }, anchors, 2);
    expect(hit?.target.ref.nodeIndex).toBe(0);
  });

  it("returns null when nothing is within threshold", () => {
    const anchors = collectAnchors(docWithOpenPath());
    expect(findSnap({ x: 100, y: 100 }, anchors, 2)).toBeNull();
  });
});

describe("isCloseLoop", () => {
  it("detects dragging one endpoint onto the other of the same open subpath", () => {
    const doc = docWithOpenPath();
    const dragged: NodeRef = { pathIndex: 0, subpathIndex: 0, nodeIndex: 2 };
    const anchors = collectAnchors(doc, dragged);
    // snap the dragged endpoint near the start node (0,0)
    const hit = findSnap({ x: 0.4, y: 0.3 }, anchors, 2);
    expect(hit).not.toBeNull();
    expect(isCloseLoop(dragged, hit!.target, doc)).toBe(true);
  });

  it("is false when snapping to a non-endpoint", () => {
    const doc = docWithOpenPath();
    const dragged: NodeRef = { pathIndex: 0, subpathIndex: 0, nodeIndex: 2 };
    const anchors = collectAnchors(doc, dragged);
    const hit = findSnap({ x: 10, y: 0 }, anchors, 2); // the middle node
    expect(isCloseLoop(dragged, hit!.target, doc)).toBe(false);
  });
});

describe("snapToGrid", () => {
  it("rounds to the nearest grid intersection", () => {
    expect(snapToGrid({ x: 11, y: 4 }, 10)).toEqual({ x: 10, y: 0 });
    expect(snapToGrid({ x: 16, y: 15 }, 10)).toEqual({ x: 20, y: 20 });
  });
});
