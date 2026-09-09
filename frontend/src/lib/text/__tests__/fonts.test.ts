import { describe, expect, it } from "vitest";

import type { TextInfo } from "$lib/model/types";

import { bestFace, type FaceInfo } from "../fonts";

/** A label asking for a given weight/style; nothing else here reads the other fields. */
function label(weight: string, style = "normal"): TextInfo {
  return {
    uid: "u",
    name: "",
    text: "Hi",
    family: "Helvetica",
    weight,
    style,
    fontSize: 16,
    x: 0,
    y: 0,
    letterSpacing: 0,
    anchor: "start",
  };
}

const face = (index: number, style: string, weight: number, italic = false): FaceInfo => ({
  index,
  family: "Helvetica",
  style,
  weight,
  italic,
});

// A picked `.ttc` arrives as bytes and a filename — nothing in the file says which of its faces
// the label wanted, so this is the only thing standing between a bold heading and regular
// outlines that look like a shaping bug.
describe("bestFace", () => {
  const collection = [
    face(0, "Regular", 400),
    face(1, "Bold", 700),
    face(2, "Italic", 400, true),
    face(3, "Bold Italic", 700, true),
  ];

  it("matches slant and weight together", () => {
    expect(bestFace(collection, label("normal"))?.index).toBe(0);
    expect(bestFace(collection, label("bold"))?.index).toBe(1);
    expect(bestFace(collection, label("normal", "italic"))?.index).toBe(2);
    expect(bestFace(collection, label("700", "italic"))?.index).toBe(3);
  });

  it("reads numeric weights the way CSS does", () => {
    // 600+ is bold territory; below it is not.
    expect(bestFace(collection, label("600"))?.index).toBe(1);
    expect(bestFace(collection, label("500"))?.index).toBe(0);
    // `oblique` slants like italic.
    expect(bestFace(collection, label("normal", "oblique"))?.index).toBe(2);
  });

  it("picks the nearest weight among faces on the same side of bold", () => {
    const family = [face(0, "Light", 300), face(1, "Regular", 400), face(2, "Medium", 500)];
    expect(bestFace(family, label("300"))?.index).toBe(0);
    expect(bestFace(family, label("500"))?.index).toBe(2);
  });

  it("returns the only face of a plain file, whatever was asked", () => {
    const single = [face(0, "Regular", 400)];
    expect(bestFace(single, label("bold", "italic"))?.index).toBe(0);
  });

  it("has nothing to offer for a file with no readable faces", () => {
    // What a `.woff2` looks like from here: compressed, so the core reads no faces out of it.
    expect(bestFace([], label("normal"))).toBeNull();
  });
});
