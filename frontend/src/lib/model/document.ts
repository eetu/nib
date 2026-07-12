import { SVGPathData } from "svg-pathdata";

import { parsePathD, pathToD } from "./path";
import type { PathElement, SvgDocument, ViewBox } from "./types";

const DEFAULT_VIEWBOX: ViewBox = { minX: 0, minY: 0, width: 100, height: 100 };

/** Presentation attributes the STYLE panel can read/edit on any path. Parsed
 *  from imported paths so they can be styled + reset. */
export const STYLE_KEYS = [
  "fill",
  "stroke",
  "stroke-width",
  "opacity",
  "fill-opacity",
  "stroke-opacity",
  "stroke-linecap",
  "stroke-linejoin",
  "stroke-dasharray",
];

function parseStyleAttrs(el: Element): Record<string, string> {
  const attrs: Record<string, string> = {};
  for (const key of STYLE_KEYS) {
    const v = el.getAttribute(key);
    if (v !== null) attrs[key] = v;
  }
  return attrs;
}

function readViewBox(svg: SVGSVGElement, paths: PathElement[]): ViewBox {
  const vb = svg.getAttribute("viewBox");
  if (vb) {
    const nums = vb
      .trim()
      .split(/[\s,]+/)
      .map(Number);
    if (nums.length === 4 && nums.every((n) => Number.isFinite(n)) && nums[2] > 0 && nums[3] > 0) {
      return { minX: nums[0], minY: nums[1], width: nums[2], height: nums[3] };
    }
  }
  const w = parseFloat(svg.getAttribute("width") ?? "");
  const h = parseFloat(svg.getAttribute("height") ?? "");
  if (Number.isFinite(w) && Number.isFinite(h) && w > 0 && h > 0) {
    return { minX: 0, minY: 0, width: w, height: h };
  }
  return boundsOf(paths) ?? DEFAULT_VIEWBOX;
}

/** Fallback viewBox from the union of all path bounds, padded 5%. */
function boundsOf(paths: PathElement[]): ViewBox | null {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const p of paths) {
    if (!p.originalD) continue;
    try {
      const b = new SVGPathData(p.originalD).toAbs().getBounds();
      minX = Math.min(minX, b.minX);
      minY = Math.min(minY, b.minY);
      maxX = Math.max(maxX, b.maxX);
      maxY = Math.max(maxY, b.maxY);
    } catch {
      // ignore unparseable paths for bounds
    }
  }
  if (!Number.isFinite(minX) || maxX <= minX || maxY <= minY) return null;
  const padX = (maxX - minX) * 0.05;
  const padY = (maxY - minY) * 0.05;
  return {
    minX: minX - padX,
    minY: minY - padY,
    width: maxX - minX + padX * 2,
    height: maxY - minY + padY * 2,
  };
}

/** Parse an SVG source string into the editable document model. Throws on
 *  markup that has no <svg> root or fails to parse. */
export function parseSvg(source: string): SvgDocument {
  const doc = new DOMParser().parseFromString(source, "image/svg+xml");
  const parseError = doc.querySelector("parsererror");
  if (parseError) {
    throw new Error(`could not parse SVG: ${parseError.textContent?.trim() ?? "invalid markup"}`);
  }
  const svg = doc.querySelector("svg");
  if (!svg) throw new Error("no <svg> root element found");

  // Opening <path …> tags in source order — the anchors for surgical rewrites.
  // Same order as querySelectorAll("path"), so index-aligned.
  const tags = [...source.matchAll(/<path\b[^>]*>/gi)].map((m) => m[0]);

  const pathEls = Array.from(svg.querySelectorAll("path"));
  const paths: PathElement[] = pathEls.map((el, index) => {
    const originalD = el.getAttribute("d") ?? "";
    return {
      id: el.getAttribute("id") || `path-${index}`,
      index,
      originalD,
      originalTag: tags[index],
      attributes: parseStyleAttrs(el),
      subpaths: parsePathD(originalD),
      edited: false,
    };
  });

  return { source, viewBox: readViewBox(svg as SVGSVGElement, paths), paths };
}

/**
 * Serialize the document back to an SVG string. Everything is preserved
 * byte-for-byte except the `d` of paths the user actually edited, which are
 * spliced in place. Paths are located in document order (from a moving cursor)
 * so duplicate `d` values still map to the right element.
 */
function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Set (or insert) one attribute in a `<path …>` tag string, preserving the
 *  rest of the tag byte-for-byte. */
function withAttr(tag: string, key: string, value: string): string {
  const re = new RegExp(`(\\s${escapeRegExp(key)}\\s*=\\s*)(["'])[\\s\\S]*?\\2`);
  if (re.test(tag)) return tag.replace(re, `$1$2${value}$2`);
  return tag.replace(/\s*\/?>$/, (close) => ` ${key}="${value}"${close}`);
}

function styleOverridden(p: PathElement): boolean {
  return !!p.styleOverride && Object.keys(p.styleOverride).length > 0;
}

export function serializeSvg(doc: SvgDocument, precision = 3): string {
  const src = doc.source;
  let out = "";
  let cursor = 0;
  for (const p of doc.paths) {
    if (p.added || !p.originalTag) continue; // added paths handled below
    const idx = src.indexOf(p.originalTag, cursor);
    if (idx === -1) continue; // can't locate — leave untouched
    const end = idx + p.originalTag.length;
    if (p.deleted) {
      out += src.slice(cursor, idx); // drop the tag
    } else if (p.edited || p.renamed || styleOverridden(p)) {
      // Rewrite only the changed d / id / style attributes, in place in the tag.
      let tag = p.originalTag;
      if (p.edited) tag = withAttr(tag, "d", pathToD(p.subpaths, precision));
      if (p.renamed) tag = withAttr(tag, "id", p.id);
      for (const [k, v] of Object.entries(p.styleOverride ?? {})) tag = withAttr(tag, k, v);
      out += src.slice(cursor, idx) + tag;
    } else {
      out += src.slice(cursor, end); // verbatim
    }
    cursor = end;
  }
  out += src.slice(cursor);
  return appendDrawnPaths(out, doc, precision);
}

function attrsToString(attrs?: Record<string, string>): string {
  if (!attrs) return "";
  return Object.entries(attrs)
    .map(([k, v]) => ` ${k}="${v}"`)
    .join("");
}

/** Insert in-app-drawn paths (no source location) just before the closing
 *  </svg>, so they become part of the exported markup. */
function appendDrawnPaths(out: string, doc: SvgDocument, precision: number): string {
  const drawn = doc.paths
    .filter((p) => p.added && !p.deleted && p.subpaths.some((sp) => sp.nodes.length >= 2))
    .map((p) => {
      const id = p.renamed ? ` id="${p.id}"` : "";
      return `  <path${id} d="${pathToD(p.subpaths, precision)}"${attrsToString(p.attributes)} />`;
    })
    .join("\n");
  if (!drawn) return out;
  const close = out.lastIndexOf("</svg>");
  return close === -1 ? `${out}\n${drawn}` : `${out.slice(0, close)}${drawn}\n${out.slice(close)}`;
}
