// Parsing + byte-for-byte serialization now live in the Rust core (core/src/model/document.rs);
// the client only needs the list of presentation attributes it projects onto rendered
// elements. Kept in sync with the core's STYLE_KEYS.
export const STYLE_KEYS = [
  "fill",
  "fill-rule",
  "stroke",
  "stroke-width",
  "opacity",
  "fill-opacity",
  "stroke-opacity",
  "stroke-linecap",
  "stroke-linejoin",
  "stroke-dasharray",
];
