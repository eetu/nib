-- Phase C model refactor: the native document model (JSON) becomes the source of truth; SVG is a
-- cached export. Existing rows keep their `svg`; the model is empty until first opened (the session
-- imports the svg → model on open and persists it). New edits write both (model = truth, svg = cache).
alter table projects add column model text not null default '';
