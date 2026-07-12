import { describe, expect, it } from "vitest";

import { parseSvg, serializeSvg } from "../document";

const SAMPLE = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <rect x="0" y="0" width="100" height="100" fill="#eee"/>
  <path id="a" d="M 10 10 L 90 10 L 90 90" fill="none" stroke="black"/>
  <path d="M 20 20 L 80 80" stroke="red"/>
</svg>`;

describe("parseSvg", () => {
  it("reads the viewBox and enumerates paths in document order", () => {
    const doc = parseSvg(SAMPLE);
    expect(doc.viewBox).toEqual({ minX: 0, minY: 0, width: 100, height: 100 });
    expect(doc.paths).toHaveLength(2);
    expect(doc.paths[0].id).toBe("a");
    expect(doc.paths[1].id).toBe("path-1");
    expect(doc.paths[0].index).toBe(0);
  });

  it("synthesizes a viewBox from width/height when absent", () => {
    const doc = parseSvg(
      `<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><path d="M0 0 L1 1"/></svg>`,
    );
    expect(doc.viewBox).toEqual({ minX: 0, minY: 0, width: 40, height: 30 });
  });

  it("throws on markup with no svg root", () => {
    expect(() => parseSvg("<div>not svg</div>")).toThrow();
  });
});

describe("serializeSvg", () => {
  it("preserves the source byte-for-byte when nothing is edited", () => {
    const doc = parseSvg(SAMPLE);
    expect(serializeSvg(doc)).toBe(SAMPLE);
  });

  it("parses imported path style attributes", () => {
    const doc = parseSvg(SAMPLE);
    expect(doc.paths[0].attributes).toMatchObject({ fill: "none", stroke: "black" });
  });

  it("applies a style override to an imported path, preserving the rest of its tag", () => {
    const doc = parseSvg(SAMPLE);
    doc.paths[0].styleOverride = { stroke: "red", "stroke-width": "3" };
    const out = serializeSvg(doc);
    expect(out).toContain('stroke="red"'); // replaced in place
    expect(out).not.toContain('stroke="black"');
    expect(out).toContain('stroke-width="3"'); // inserted (wasn't present)
    expect(out).toContain('id="a"'); // untouched attrs preserved
    expect(out).toContain('fill="none"');
    expect(out).toContain('d="M 10 10 L 90 10 L 90 90"'); // d untouched
    // the second path + rect are still verbatim
    expect(out).toContain(`<path d="M 20 20 L 80 80" stroke="red"/>`);
  });

  it("splices only the edited path's d, leaving the rest untouched", () => {
    const doc = parseSvg(SAMPLE);
    // edit the second path: move its endpoint
    const p = doc.paths[1];
    p.subpaths[0].nodes[1].point = { x: 70, y: 70 };
    p.edited = true;

    const out = serializeSvg(doc);
    // first path + rect + attributes are unchanged
    expect(out).toContain(`<rect x="0" y="0" width="100" height="100" fill="#eee"/>`);
    expect(out).toContain(`d="M 10 10 L 90 10 L 90 90"`);
    // second path's d is rewritten
    expect(out).toContain(`d="M 20 20 L 70 70"`);
    expect(out).not.toContain(`d="M 20 20 L 80 80"`);
  });

  it("maps duplicate d values to the right element by document order", () => {
    const dup = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0 0 L5 5"/><path d="M0 0 L5 5"/></svg>`;
    const doc = parseSvg(dup);
    doc.paths[1].subpaths[0].nodes[1].point = { x: 9, y: 9 };
    doc.paths[1].edited = true;
    const out = serializeSvg(doc);
    // exactly one occurrence rewritten (the second), the first preserved
    expect(out).toContain(`<path d="M0 0 L5 5"/><path d="M 0 0 L 9 9"/>`);
  });
});
