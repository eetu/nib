import { existsSync } from "node:fs";

import { expect, type Locator, test } from "@playwright/test";

/**
 * The selection box's own proportions and tilt, read from its four corners.
 *
 * The box is a polygon of screen-space points rather than a rect plus a rotation, so this measures
 * what a person sees: the length of its top edge (`w`), of its side (`h`), the `aspect` between
 * them, and the angle the top edge runs at (`deg`).
 *
 * Compare rotations by `aspect` and `deg`, never by `w`/`h` alone: a box that *turns* holds its
 * proportions and changes its angle, while one re-derived from mid-rotation bounds squashes from
 * wide to tall — which is the bug these tests exist for — and unlike the absolute lengths, the
 * ratio doesn't move when the view's content-aware fit settles a frame later.
 */
async function boxGeom(
  box: Locator,
): Promise<{ w: number; h: number; aspect: number; deg: number }> {
  const pts = (await box.getAttribute("points")) ?? "";
  const [nw, ne, se] = pts
    .trim()
    .split(/\s+/)
    .map((pair) => {
      const [x, y] = pair.split(",").map(Number);
      return { x, y };
    });
  const w = Math.hypot(ne.x - nw.x, ne.y - nw.y);
  const h = Math.hypot(se.x - ne.x, se.y - ne.y);
  return { w, h, aspect: w / h, deg: (Math.atan2(ne.y - nw.y, ne.x - nw.x) * 180) / Math.PI };
}

// Each test boots a fresh browser context (empty localStorage), which would trip the one-time
// first-run interface chooser and cover the canvas. Seed a prior UI-level pick so every test boots
// as a returning user; the dedicated first-run test below clears it to exercise the chooser.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    try {
      localStorage.setItem("nib:uiLevel", "advanced");
    } catch {
      /* storage unavailable — non-fatal */
    }
  });
});

// End-to-end proof that the Svelte UI drives the Rust/WASM engine correctly: boot → load →
// render → draw → undo, asserting no console/page errors throughout. This is the check that
// the document store's delegation to nib-core actually works in a browser (Phase A5).
test("boots the core, loads a sample, draws, and undoes without errors", { tag: "@cross" }, async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");

  // The WASM core booted → its version is stamped on <html> by the root layout.
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });

  // Empty state → load the built-in sample (load → WASM parse → render effect).
  await page.getByRole("button", { name: "load sample" }).click();
  const artwork = page.locator("svg.canvas g.artwork path").first();
  await expect(artwork).toBeAttached();
  // The canvas renders paths from the model (normalized `d`), so match tolerantly.
  await expect(artwork).toHaveAttribute("d", /M\s*40[\s,]+120/);

  // Draw a two-node path with the pen (beginPath + appendNode ops). Drawn paths now live in the
  // tree, so they render in g.artwork alongside the imported sample → assert the count grew by 1.
  const beforeDraw = await page.locator("svg.canvas g.artwork path").count();
  await page.keyboard.press("p");
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  await page.mouse.click(box.x + box.width * 0.3, box.y + box.height * 0.35);
  await page.mouse.click(box.x + box.width * 0.6, box.y + box.height * 0.6);
  await page.keyboard.press("Escape");
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(beforeDraw + 1);

  // Undo the last commit (exercises the WASM history) — the app stays responsive.
  await page.keyboard.press("Meta+z");
  await expect(page.locator("svg.canvas")).toBeVisible();

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("draws a rectangle with the rect shape tool", { tag: "@cross" }, async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });

  // Start a blank drawing so the canvas mounts, pick the rect tool (shortcut), drag it out.
  await page.getByRole("button", { name: "new drawing" }).click();
  await expect(page.locator("svg.canvas")).toBeVisible();
  await page.keyboard.press("r");
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  await page.mouse.move(box.x + box.width * 0.35, box.y + box.height * 0.35);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.6);
  await page.mouse.up();

  // One path (the rectangle) rendered from the tree, round-tripping to a 4-corner closed `d`.
  const rect = page.locator("svg.canvas g.artwork path");
  await expect(rect).toHaveCount(1);
  await expect(rect).toHaveAttribute("d", /Z$/);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("rect tool draws rounded corners when a corner radius is set", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.getByRole("button", { name: "new drawing" }).click();
  await expect(page.locator("svg.canvas")).toBeVisible();

  // Pick the rect tool → the "new shape style" panel exposes the corner radius; set it.
  await page.keyboard.press("r");
  await page.getByRole("spinbutton", { name: "corner radius" }).fill("8");

  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.3);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.7, box.y + box.height * 0.7);
  await page.mouse.up();

  // The drawn path has curve commands (rounded corners), not a plain 4-line box.
  const rect = page.locator("svg.canvas g.artwork path");
  await expect(rect).toHaveCount(1);
  await expect(rect).toHaveAttribute("d", /C/);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("New drawing creates a blank document from the top bar", { tag: "@cross" }, async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });

  // The empty state is up; New in the top bar makes a blank document.
  await page.locator("header").getByRole("button", { name: "new", exact: true }).click();
  await expect(page.locator("svg.canvas")).toBeVisible();
  await expect(page.locator("header .name")).toHaveText("untitled.svg");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("the command palette opens and runs an action", { tag: "@cross" }, async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.getByRole("button", { name: "new drawing" }).click();

  await page.keyboard.press("Meta+k");
  const dialog = page.getByRole("dialog", { name: "Command palette" });
  await expect(dialog).toBeVisible();
  await page.locator(".palette .q").fill("fit");
  await page.keyboard.press("Enter");
  await expect(dialog).toBeHidden();

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("double-click enters node editing — anchors appear only then", { tag: "@cross" }, async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M20 20 H80 V80 H20 Z" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await expect(page.locator("svg.canvas g.artwork path")).toBeAttached();

  await page.keyboard.press("v");
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;

  // Object mode: selecting shows the transform box but NO editable anchors.
  await page.mouse.click(cx, cy);
  await expect(page.locator("svg.canvas g.overlay polygon.sel-box")).toBeAttached();
  await expect(page.locator("svg.canvas g.overlay .anchor")).toHaveCount(0);

  // Double-click enters node editing → the square's four anchors appear.
  await page.mouse.dblclick(cx, cy);
  await expect(page.locator("svg.canvas g.overlay .anchor")).toHaveCount(4);
  await expect(page.locator("svg.canvas g.overlay polygon.sel-box")).toHaveCount(0);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("shift-selecting two paths enables align", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><path d="M10 10 H40 V40 H10 Z" fill="#f00"/><path d="M60 60 H90 V90 H60 Z" fill="#00f"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Select the first path, shift-select the second → a multi-selection → the arrange panel.
  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(2);
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });
  await expect(page.getByRole("heading", { name: /arrange/ })).toBeVisible();

  // Align left: the second rect (minX 60) moves onto the first's left edge (10) → its d changes.
  const before = await page.locator("svg.canvas g.artwork path").nth(1).getAttribute("d");
  await page.getByTitle("align left").click();
  await expect(page.locator("svg.canvas g.artwork path").nth(1)).not.toHaveAttribute(
    "d",
    before ?? "",
  );

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("multi-select shows group transform handles and scales all shapes together", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M10 10 H40 V40 H10 Z" fill="#f00"/><path d="M60 60 H90 V90 H60 Z" fill="#00f"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Multi-select both shapes via the layers list.
  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(2);
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });

  // The union box now carries the 8 resize handles (multi-select used to be move-only).
  await expect(page.locator("svg.canvas g.overlay polygon.sel-box")).toBeAttached();
  const handles = page.locator("svg.canvas g.overlay rect.xf-handle");
  await expect(handles).toHaveCount(8);

  // Drag the SE corner (handlePoints index 4) outward → BOTH shapes scale as one group.
  const paths = page.locator("svg.canvas g.artwork path");
  const before0 = await paths.nth(0).getAttribute("d");
  const before1 = await paths.nth(1).getAttribute("d");
  const se = await handles.nth(4).boundingBox();
  if (!se) throw new Error("no SE handle");
  await page.mouse.move(se.x + se.width / 2, se.y + se.height / 2);
  await page.mouse.down();
  await page.mouse.move(se.x + 60, se.y + 60);
  await page.mouse.up();
  await expect(paths.nth(0)).not.toHaveAttribute("d", before0 ?? "");
  await expect(paths.nth(1)).not.toHaveAttribute("d", before1 ?? "");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("layers: group two shapes, then hide the group", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.getByRole("button", { name: "new drawing" }).click();
  await expect(page.locator("svg.canvas")).toBeVisible();

  // Draw two rectangles.
  await page.keyboard.press("r");
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  await page.mouse.move(box.x + box.width * 0.25, box.y + box.height * 0.25);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.45, box.y + box.height * 0.45);
  await page.mouse.up();
  await page.mouse.move(box.x + box.width * 0.55, box.y + box.height * 0.55);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.75, box.y + box.height * 0.75);
  await page.mouse.up();
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(2);

  // Select both shapes in the layers list, then group them.
  await page.keyboard.press("v");
  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(2);
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "group selection" }).click();
  await expect(page.locator(".layerlist .grouphead")).toHaveCount(1);

  // Hiding the group removes its shapes from the render.
  await page.getByRole("button", { name: "toggle group visibility" }).click();
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(0);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("gradients: convert a shape's fill to a linear gradient", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.getByRole("button", { name: "new drawing" }).click();

  // Draw a rectangle, then switch to select → it's object-selected (transform box).
  await page.keyboard.press("r");
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.3);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.65, box.y + box.height * 0.65);
  await page.mouse.up();
  await page.keyboard.press("v");
  await expect(page.locator("svg.canvas g.overlay polygon.sel-box")).toBeAttached();

  // Fill → linear gradient. A paint with no fill has no kind to change, so it's switched on
  // first — the kind list only exists for a paint that exists (Pixelmator's split).
  await page.getByLabel("fill on").click();
  await page.getByLabel("fill kind").selectOption("linear");
  await expect(page.locator("svg.canvas defs linearGradient")).toHaveCount(1);
  await expect(page.locator("svg.canvas g.artwork path")).toHaveAttribute("fill", /url\(#grad-/);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("copy style transfers a fill from one path to another", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><path d="M10 10 H40 V40 H10 Z" fill="#ff0000"/><path d="M60 60 H90 V90 H60 Z" fill="#0000ff"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Rows are top-of-stack first, so nth(1) is the red path (index 0) and nth(0) is the blue
  // (index 1). Copy red's style, then paste it onto blue → blue's fill becomes red.
  const rows = page.locator(".layerlist .row-btn");
  await rows.nth(1).click();
  await page.getByRole("button", { name: "copy style" }).click();
  await rows.nth(0).click();
  await page.getByRole("button", { name: "paste style" }).click();
  await expect(page.locator("svg.canvas g.artwork path").nth(1)).toHaveAttribute("fill", "#ff0000");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("simplify reduces a path's node count", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 20"><path d="M0 5 L10 5 L20 5 L30 5 L40 5 L50 5" fill="none" stroke="#000"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const artwork = page.locator("svg.canvas g.artwork path");
  const before = await artwork.getAttribute("d");
  await page.locator(".layerlist .row-btn").first().click();
  await page.keyboard.press("Meta+k");
  await page.locator(".palette .q").fill("simplify");
  await page.keyboard.press("Enter");
  // The collinear midpoints collapse → the d shortens.
  await expect(artwork).not.toHaveAttribute("d", before ?? "");
  const after = await artwork.getAttribute("d");
  expect((after ?? "").length).toBeLessThan((before ?? "").length);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("outline stroke turns a stroked line into a filled shape", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M10 10 L60 50" stroke="#ff0000" stroke-width="6" fill="none"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  await page.locator(".layerlist .row-btn").first().click();
  await page.keyboard.press("Meta+k");
  await page.locator(".palette .q").fill("outline");
  await page.keyboard.press("Enter");

  // The stroked line is replaced by a fill shape whose fill is the old stroke colour (the source
  // is soft-deleted, so only the outline paints in the tree).
  const drawn = page.locator("svg.canvas g.artwork path");
  await expect(drawn).toHaveCount(1);
  await expect(drawn).toHaveAttribute("fill", "#ff0000");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("offset path adds a second, larger path", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M30 30 H70 V70 H30 Z" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");
  await page.locator(".layerlist .row-btn").first().click();

  await page.keyboard.press("Meta+k");
  await page.locator(".palette .q").fill("offset path outward");
  await page.keyboard.press("Enter");
  // The offset result is a new path (source kept) → two paths in the tree render.
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(2);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("skew shears the selected path", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M20 20 H80 V80 H20 Z" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");
  await page.locator(".layerlist .row-btn").first().click();

  const artwork = page.locator("svg.canvas g.artwork path");
  const before = await artwork.getAttribute("d");
  const skewX = page.locator(".pairrow").filter({ hasText: "skew" }).locator("input").first();
  await skewX.fill("20");
  await skewX.press("Tab");
  await expect(artwork).not.toHaveAttribute("d", before ?? "");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("combine merges two paths into one compound path", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  // a line + a detached dome (two separate paths)
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 40"><path d="M0 30 L100 30" fill="none" stroke="#000"/><path d="M35 30 Q50 5 65 30" fill="none" stroke="#000"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(2);
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "compound path" }).click();
  // The two paths become one row whose d holds both subpaths (two M commands).
  await expect(rows).toHaveCount(1);
  const d = await page.locator("svg.canvas g.artwork path").getAttribute("d");
  expect((d ?? "").match(/M/g)?.length ?? 0).toBeGreaterThanOrEqual(2);

  // Release splits it back into two independent, individually-styleable paths.
  await rows.nth(0).click();
  await page.getByRole("button", { name: "release compound" }).click();
  await expect(rows).toHaveCount(2);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("boolean union combines two shapes into one", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.getByRole("button", { name: "new drawing" }).click();

  // Two overlapping rectangles.
  await page.keyboard.press("r");
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.3);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.55, box.y + box.height * 0.55);
  await page.mouse.up();
  await page.mouse.move(box.x + box.width * 0.45, box.y + box.height * 0.45);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.7, box.y + box.height * 0.7);
  await page.mouse.up();
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(2);

  // Select both, then union → one result path replaces them.
  await page.keyboard.press("v");
  const rows = page.locator(".layerlist .row-btn");
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "union", exact: true }).click();
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(1);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("live boolean keeps operands editable and recomputes the result", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  // Two overlapping filled squares.
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 120"><path d="M10 10 H70 V70 H10 Z" fill="#3b82f6"/><path d="M50 50 H110 V110 H50 Z" fill="#ef4444"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(2);
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });

  // Turn on live (non-destructive) mode, then subtract → a live boolean group.
  await page.getByLabel("live (non-destructive)").check();
  await page.getByRole("button", { name: "subtract", exact: true }).click();

  // The computed result renders (in the tree, from the <g booleanOp> node), and BOTH operands
  // survive as editable rows (non-destructive — vs the destructive boolean which collapses to one).
  const result = page.locator("svg.canvas g.artwork path");
  await expect(result).toHaveCount(1);
  await expect(rows).toHaveCount(2);

  // Reshape an operand (nudge it) → the result recomputes live (its `d` changes).
  const beforeD = await result.getAttribute("d");
  await rows.nth(0).click();
  for (let i = 0; i < 8; i++) await page.keyboard.press("ArrowRight");
  await expect(result).not.toHaveAttribute("d", beforeD ?? "");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("basic UI level hides advanced tools; advanced restores them", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });

  // Default is advanced → the shapes flyout is present.
  const shapes = page.getByRole("button", { name: "shapes tools" });
  await expect(shapes).toBeVisible();

  // Switch to basic via settings → shape primitives disappear from the rail.
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "basic", exact: true }).click();
  await page.getByRole("button", { name: "done" }).click();
  await expect(shapes).toHaveCount(0);

  // The advanced-tool shortcut is inert in basic (pressing "r" doesn't switch to rect).
  await page.keyboard.press("r");
  await expect(shapes).toHaveCount(0);

  // Back to advanced restores them.
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "advanced", exact: true }).click();
  await page.getByRole("button", { name: "done" }).click();
  await expect(shapes).toBeVisible();

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("first run shows the interface chooser; picking basic applies + persists", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  // Undo the beforeEach seed so this boots as a genuine first run (later init scripts win).
  await page.addInitScript(() => {
    try {
      localStorage.removeItem("nib:uiLevel");
    } catch {
      /* non-fatal */
    }
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });

  // The one-time chooser is up.
  const chooser = page.getByRole("dialog", { name: "Choose your workspace" });
  await expect(chooser).toBeVisible();

  // Pick basic → chooser closes and the advanced-only shapes flyout is absent.
  await chooser.getByRole("button", { name: /basic/i }).click();
  await expect(chooser).toHaveCount(0);
  await expect(page.getByRole("button", { name: "shapes tools" })).toHaveCount(0);

  // The pick persisted, so the chooser won't return next visit.
  await expect.poll(() => page.evaluate(() => localStorage.getItem("nib:uiLevel"))).toBe("basic");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("an imported <rect> is editable and stays a <rect> when moved", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="20" y="20" width="40" height="40" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // The <rect> projects into the model as an editable path row (Phase E: primitives editable).
  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(1);

  // The whole document renders declaratively from the tree, so the rect draws as a <path> in the
  // artwork (from the model) — no <rect> DOM node on the canvas.
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(1);
  await expect(page.locator("svg.canvas g.artwork rect")).toHaveCount(0);

  // Select + nudge → the whole rect moves; a form-preserving move keeps it a <rect> on export.
  await rows.nth(0).click();
  for (let i = 0; i < 3; i++) await page.keyboard.press("ArrowRight");

  // Source (= export) still has a <rect> (moved), not a <path> — clean markup preserved.
  await page.getByRole("button", { name: "source" }).click();
  const src = await page.locator(".sourceview textarea").inputValue();
  expect(src).toContain("<rect");
  expect(src).toContain('x="23"'); // nudged +3
  expect(src).not.toContain("<path");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("rotate turns a shape 90° about its centre (bbox w/h swap)", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  // A wide rect (40×10) so a 90° turn visibly swaps width/height.
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="20" y="35" width="40" height="10" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Select the rect → the transform panel's size row reads 40 × 10.
  await page.locator(".layerlist .row-btn").first().click();
  const sizeInputs = page.locator(".pairrow", { hasText: "size" }).locator("input");
  expect(Math.round(Number(await sizeInputs.nth(0).inputValue()))).toBe(40);
  expect(Math.round(Number(await sizeInputs.nth(1).inputValue()))).toBe(10);

  // Rotate 90° clockwise about its centre → width/height swap to 10 × 40.
  await page.locator('button[title="rotate 90° clockwise"]').click();
  expect(Math.round(Number(await sizeInputs.nth(0).inputValue()))).toBe(10);
  expect(Math.round(Number(await sizeInputs.nth(1).inputValue()))).toBe(40);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("flip horizontal mirrors a shape about its centre", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  // A right-triangle (asymmetric), bbox 0-40 → centre x=20.
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M 0 0 L 40 0 L 0 40 Z" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  await page.locator(".layerlist .row-btn").first().click();
  await page.getByRole("button", { name: "flip h" }).click();

  // Mirrored about x=20: (0,0)→(40,0), (40,0)→(0,0), (0,40)→(40,40).
  await page.getByRole("button", { name: "source" }).click();
  const src = await page.locator(".sourceview textarea").inputValue();
  expect(src).toContain("40 40"); // the third vertex mirrored to the right

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("select-all (⌘A) then delete clears every shape", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="5" y="5" width="20" height="20"/><rect x="40" y="40" width="20" height="20"/><circle cx="80" cy="20" r="10"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(3);
  // Select all → delete removes the whole selection.
  await page.locator("svg.canvas").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Meta+a");
  await page.keyboard.press("Delete");
  await expect(rows).toHaveCount(0);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("send to back (⌘⇧[) moves the top shape below the others", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  // "b" is drawn after "a", so it's on top (last in document order).
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect id="a" x="0" y="0" width="30" height="30"/><rect id="b" x="10" y="10" width="30" height="30"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // The layers list is reversed (top of stack first) → the first row is "b". Select + send to back.
  await page.locator(".layerlist .row-btn").first().click();
  await page.keyboard.press("Meta+Shift+BracketLeft");

  await page.getByRole("button", { name: "source" }).click();
  const src = await page.locator(".sourceview textarea").inputValue();
  expect(src.indexOf('id="b"')).toBeLessThan(src.indexOf('id="a"')); // b now behind a

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("a locked shape can't be selected on the canvas", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  // A near-full-canvas rect so a centre click reliably lands on it.
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="5" y="5" width="90" height="90" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  const centre = { x: box.x + box.width * 0.5, y: box.y + box.height * 0.5 };
  const row = page.locator(".layerlist .row-btn").first();

  // Baseline: clicking the shape selects it.
  await page.mouse.click(centre.x, centre.y);
  await expect(row).toHaveClass(/active/);

  // Lock it (deselects), then a click no longer selects it.
  await page.getByRole("button", { name: "toggle lock" }).click();
  await expect(row).not.toHaveClass(/active/);
  await page.mouse.click(centre.x, centre.y);
  await expect(row).not.toHaveClass(/active/);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("text tool places an editable <text> label", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.getByRole("button", { name: "new drawing" }).click();
  await expect(page.locator("svg.canvas")).toBeVisible();

  // Pick the text tool + click → a <text> lands and is selected.
  await page.keyboard.press("t");
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  await page.mouse.click(box.x + box.width * 0.5, box.y + box.height * 0.5);

  // Rendered as a real <text> in the artwork, and the Inspector edits its content.
  await expect(page.locator("svg.canvas g.artwork text")).toHaveCount(1);
  await expect(page.getByRole("textbox", { name: "text content" })).toHaveValue("Text");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("eyedropper samples one shape's fill onto the selection", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  // A = a small red corner; B = a big blue rect filling most of the (square) viewBox, so a canvas
  // CENTRE click reliably lands on B regardless of the canvas aspect ratio / letterboxing.
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect id="a" x="0" y="0" width="12" height="12" fill="#ff0000"/><rect id="b" x="12" y="12" width="76" height="76" fill="#0000ff"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");

  // Select A via the layers panel (deterministic; A is the lower row — layers show top-of-stack
  // first, so B is row 0 and A is row 1). Then eyedropper-click centre (B) → A takes B's blue.
  await page.locator(".layerlist .row-btn").nth(1).click();
  await page.keyboard.press("i");
  await page.mouse.click(box.x + box.width * 0.5, box.y + box.height * 0.5);

  await page.getByRole("button", { name: "source" }).click();
  const src = await page.locator(".sourceview textarea").inputValue();
  expect(src).not.toContain("#ff0000"); // A's red is gone
  expect(src.match(/#0000ff/g)?.length).toBe(2); // both rects blue now

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

// The eyedropper answers "what colour?", so its button belongs beside the colour it fills in —
// one per paint, which also says *which* paint the next click lands in. It left the tool rail
// (where it sat between pen and text, answering a different kind of question) but stayed a
// registered tool: same cursor, same `i` shortcut.
// Arming the eyedropper BORROWS the current tool; it doesn't switch away from it. Switching runs
// the outgoing tool's cleanup — and the pen's cleanup is "finish the path" — so picking a colour
// for the pen used to end the path you were drawing and leave you on the select tool.
test("the eyedropper borrows the current tool and hands it back", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="20" y="20" width="60" height="60" fill="#0000ff"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");

  // Arm the pen with nothing selected: the style panel is the pen's NEW-SHAPE style.
  await page.keyboard.press("p");
  const pen = page.locator(".rail").getByRole("button", { name: /^pen/ });
  await expect(pen).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("heading", { name: "new shape style" })).toBeVisible();

  // Take a stroke colour off the blue rect. (The pen's default already has a stroke —
  // `currentColor` — so the switch is on and there's a field waiting for the sample.)
  await expect(page.getByLabel("stroke on")).toHaveAttribute("aria-checked", "true");
  await page.getByLabel("stroke eyedropper").click();

  // Borrowed, not switched: the pen still reads as the selected tool and the panel is still its.
  await expect(pen).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("heading", { name: "new shape style" })).toBeVisible();

  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  await page.mouse.click(box.x + box.width * 0.5, box.y + box.height * 0.5);

  // Handed back to the pen — not dropped on select — and the colour stuck to the pen's default.
  await expect(pen).toHaveAttribute("aria-pressed", "true");
  const strokeField = page.locator(".paint").filter({ hasText: "stroke" }).locator("input.hex");
  await expect(strokeField).toHaveValue("#0000ff");

  // Drawing with it now uses the sampled stroke.
  await page.mouse.click(box.x + box.width * 0.2, box.y + box.height * 0.2);
  await page.mouse.click(box.x + box.width * 0.3, box.y + box.height * 0.3);
  await page.keyboard.press("Enter");
  await expect(page.locator('svg.canvas g.artwork path[stroke="#0000ff"]')).toHaveCount(1);

  // Escape un-arms a borrowed tool rather than falling through to the pen's own Escape.
  await page.getByLabel("stroke eyedropper").click();
  await page.keyboard.press("Escape");
  await expect(pen).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByLabel("stroke eyedropper")).not.toHaveClass(/\bon\b/);
});

test("the eyedropper lives with the colours, one per paint", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect id="a" x="0" y="0" width="12" height="12" fill="#ff0000" stroke="#111111"/><rect id="b" x="12" y="12" width="76" height="76" fill="#0000ff"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Gone from the rail...
  await expect(page.locator(".rail").getByRole("button", { name: /eyedropper/i })).toHaveCount(0);

  // ...and present once per paint in the style panel, once a shape is selected.
  await page.locator(".layerlist .row-btn").nth(1).click();
  const fillPick = page.getByLabel("fill eyedropper");
  const strokePick = page.getByLabel("stroke eyedropper");
  await expect(fillPick).toBeVisible();
  await expect(strokePick).toBeVisible();

  // Arming one lights that one only — with two on screen, which paint is waiting has to be visible.
  await strokePick.click();
  await expect(strokePick).toHaveClass(/\bon\b/);
  await expect(fillPick).not.toHaveClass(/\bon\b/);

  // And the stroke picker samples into the STROKE: A keeps its red fill, takes B's blue stroke.
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  await page.mouse.click(box.x + box.width * 0.5, box.y + box.height * 0.5);

  await page.getByRole("button", { name: "source" }).click();
  const src = await page.locator(".sourceview textarea").inputValue();
  expect(src).toContain('fill="#ff0000"'); // the fill was left alone
  expect(src).toContain('stroke="#0000ff"'); // the stroke took the sample
  expect(src).not.toContain("#111111");
});

test("drop shadow adds a filter def + references it; removing clears it", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="20" y="20" width="40" height="40" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Select the rect, then add a drop shadow.
  await page.locator(".layerlist .row-btn").first().click();
  await page.getByRole("button", { name: "+ drop shadow" }).click();

  // Source (= export) has a filter def with feDropShadow, and the rect references it.
  await page.getByRole("button", { name: "source" }).click();
  const src = await page.locator(".sourceview textarea").inputValue();
  expect(src).toContain("<filter");
  expect(src).toContain("feDropShadow");
  expect(src).toContain('filter="url(#');

  // Remove it → the rect no longer references a filter.
  await page.getByRole("button", { name: "remove drop shadow" }).click();
  const src2 = await page.locator(".sourceview textarea").inputValue();
  expect(src2).not.toContain('filter="url(#');

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("declarative render draws shapes as paths and opaque elements (text) verbatim", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><rect x="10" y="10" width="40" height="40" fill="#3b82f6"/><text x="20" y="90" font-size="12">hi</text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");

  // The whole document is rendered declaratively from the tree: the editable rect draws as a
  // <path> (from the model), while the opaque <text> renders verbatim (the fidelity path).
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(1);
  const text = page.locator("svg.canvas g.artwork text");
  await expect(text).toHaveCount(1);
  await expect(text).toHaveText("hi");
  await expect(text).toHaveAttribute("x", "20");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("declarative render keeps gradient defs functional (SVG namespace) + fill refs", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><radialGradient id="g" cx="0" cy="0" r="100" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></radialGradient></defs><rect fill="url(#g)" width="100" height="100"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");

  // The gradient def renders inside the canvas — and crucially in the SVG namespace, or the
  // browser silently ignores it and the fill shows nothing.
  const grad = page.locator("svg.canvas radialGradient#g");
  await expect(grad).toHaveCount(1);
  const ns = await grad.evaluate((el) => el.namespaceURI);
  expect(ns).toBe("http://www.w3.org/2000/svg");
  // Two stops rendered under it.
  await expect(page.locator("svg.canvas radialGradient#g stop")).toHaveCount(2);
  // The rect (drawn as a path) still references the gradient.
  await expect(page.locator('svg.canvas g.artwork path[fill="url(#g)"]')).toHaveCount(1);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("defs (clipPath/filter) render + their contents aren't editable paths", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><clipPath id="clip"><circle cx="50" cy="50" r="40"/></clipPath><filter id="blur"><feGaussianBlur stdDeviation="2"/></filter></defs><rect x="10" y="10" width="80" height="80" fill="#3b82f6" clip-path="url(#clip)"/><path d="M20 20 L80 80" stroke="#000" filter="url(#blur)"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");

  // The defs render (so clip/filter work) — clipPath + filter exist in the canvas DOM.
  await expect(page.locator("svg.canvas defs clipPath#clip")).toHaveCount(1);
  await expect(page.locator("svg.canvas defs filter#blur")).toHaveCount(1);
  // Only the two referencing shapes are editable paths — the <circle> inside the clipPath is NOT
  // projected as a top-level path/row (it's def content, opaque).
  await expect(page.locator("svg.canvas g.artwork > path")).toHaveCount(2);
  await expect(page.locator(".layerlist .row-btn")).toHaveCount(2);
  // The rect (drawn as a path) keeps its clip-path reference.
  await expect(page.locator('svg.canvas g.artwork path[clip-path="url(#clip)"]')).toHaveCount(1);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("the colour picker has an alpha channel (fill becomes 8-digit hex)", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="10" y="10" width="80" height="80" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");
  await page.locator(".layerlist .row-btn").first().click();

  // The fill paint's colour picker exposes an alpha slider; setting it makes the fill 8-digit hex.
  const alpha = page.locator(".paint").filter({ hasText: "fill" }).locator('.alpha input[type="range"]');
  await expect(alpha).toBeVisible();
  await alpha.fill("50");
  await expect(page.locator("svg.canvas g.artwork path").first()).toHaveAttribute(
    "fill",
    /^#3b82f6[0-9a-f]{2}$/i,
  );

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("a large document (hundreds of paths) loads + stays interactive", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  const N = 300;
  const rects = Array.from(
    { length: N },
    (_, i) =>
      `<rect x="${(i % 25) * 8}" y="${Math.floor(i / 25) * 8}" width="6" height="6" fill="#3b82f6"/>`,
  ).join("");
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 250 160">${rects}</svg>`);
  await page.keyboard.press("Meta+Enter");
  const paths = page.locator("svg.canvas g.artwork path");
  await expect(paths).toHaveCount(N);

  // Interaction stays responsive at scale: select a row + nudge → its d changes well under a
  // generous budget (a catastrophic O(n²) regression would blow past it).
  await page.keyboard.press("v");
  await page.locator(".layerlist .row-btn").first().click(); // top-of-stack = last doc path
  const last = paths.last();
  const d0 = await last.getAttribute("d");
  const t = Date.now();
  await page.keyboard.press("ArrowRight");
  await expect(last).not.toHaveAttribute("d", d0 ?? "");
  expect(Date.now() - t).toBeLessThan(3000);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("an SVG with a DOCTYPE/DTD (Inkscape/Illustrator) loads", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<?xml version="1.0"?>\n<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">\n<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="10" y="10" width="80" height="80" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  // It parses + renders (a DTD would previously have been rejected → nothing loaded).
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(1);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("undo then redo restores a drawn shape", { tag: "@cross" }, async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.getByRole("button", { name: "new drawing" }).click();
  await page.keyboard.press("r");
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("no canvas");
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.3);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.6);
  await page.mouse.up();
  const path = page.locator("svg.canvas g.artwork path");
  await expect(path).toHaveCount(1);

  await page.keyboard.press("v");
  const run = async (cmd: string) => {
    await page.keyboard.press("Meta+k");
    await page.locator(".palette .q").fill(cmd);
    await page.keyboard.press("Enter");
  };
  await run("Undo");
  await expect(path).toHaveCount(0);
  await run("Redo");
  await expect(path).toHaveCount(1);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("editing the SOURCE drawer re-parses the document", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="0" y="0" width="20" height="20"/></svg>`);
  await page.keyboard.press("Meta+Enter");
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(1);

  // Edit the source to two rects + apply → the canvas re-parses to two paths.
  await page.getByRole("button", { name: "source" }).click();
  await page
    .locator(".sourceview textarea")
    .fill(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="0" y="0" width="20" height="20"/><rect x="40" y="0" width="20" height="20"/></svg>`);
  await page.locator(".sourceview .apply").click();
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(2);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("flattening a live boolean group renders its operands again", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 120"><path d="M10 10 H70 V70 H10 Z" fill="#3b82f6"/><path d="M50 50 H110 V110 H50 Z" fill="#ef4444"/></svg>`);
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const rows = page.locator(".layerlist .row-btn");
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });
  await page.getByLabel("live (non-destructive)").check();
  await page.getByRole("button", { name: "subtract", exact: true }).click();
  // A live boolean group → one baked result path in the tree.
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(1);

  // Flatten via the group header menu → plain group → both operands render again.
  await page.locator(".layerlist .grouphead").click({ button: "right" });
  await page.getByRole("menuitem", { name: "flatten (plain group)" }).click();
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(2);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("drag-drop in the Layers panel reorders z-order", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect id="r1" x="0" y="0" width="20" height="20"/><rect id="r2" x="40" y="0" width="20" height="20"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const rowLi = (id: string) => page.locator(".layerlist li.pathrow").filter({ hasText: id });
  await expect(rowLi("r1")).toBeVisible();
  // Drag r1's row onto the top of r2's row → r1 moves *after* r2 in document order (the panel is
  // reversed, so the top of a row = higher z = later in the document).
  await rowLi("r1").dragTo(rowLi("r2"), { targetPosition: { x: 20, y: 2 } });

  await page.getByRole("button", { name: "source" }).click();
  const src = await page.locator(".sourceview textarea").inputValue();
  expect(src.indexOf('id="r2"')).toBeLessThan(src.indexOf('id="r1"'));

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("a <text> element is selectable and its content + attributes are editable", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text x="20" y="50" font-size="14" fill="#000000">hello</text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");

  // The <text> is a leaf row in the panel (not a shape/group), named by its own words. Selecting
  // it opens the element section (there are no shape rows — text isn't an editable path).
  const row = page.locator(".layerlist .row-btn").filter({ hasText: "hello" });
  await expect(row).toHaveCount(1);
  await row.click();
  await expect(page.getByRole("heading", { name: "text", exact: true })).toBeVisible();

  // Edit the content → the canvas <text> updates.
  const canvasText = page.locator("svg.canvas g.artwork text");
  await expect(canvasText).toHaveText("hello");
  const content = page.getByLabel("text content");
  await content.fill("world");
  await content.press("Tab");
  await expect(canvasText).toHaveText("world");

  // Edit x → the attribute updates in place, and export keeps a <text> (not a <path>).
  const xField = page.getByLabel("x", { exact: true });
  await xField.fill("40");
  await xField.press("Tab");
  await expect(canvasText).toHaveAttribute("x", "40");

  await page.getByRole("button", { name: "source" }).click();
  const src = await page.locator(".sourceview textarea").inputValue();
  expect(src).toContain("<text");
  expect(src).toContain(">world</text>");
  expect(src).toContain('x="40"');

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("a selected <text> element can be dragged on the canvas to move it", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text x="20" y="50" font-size="14">hello</text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Select via the panel row (reliable), then the overlay draws a box around the text's DOM bbox.
  await page.locator(".layerlist .row-btn").filter({ hasText: "hello" }).click();
  const selBox = page.locator("svg.canvas g.overlay polygon.sel-box");
  await expect(selBox).toBeVisible();

  // Drag inside the box → the text's x moves (drag-anywhere-in-box, forgiving of glyph gaps).
  const box = await selBox.boundingBox();
  if (!box) throw new Error("no element box");
  const t = page.locator("svg.canvas g.artwork text");
  const x0 = Number(await t.getAttribute("x"));
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 40, box.y + box.height / 2, { steps: 5 });
  await page.mouse.up();
  const x1 = Number(await t.getAttribute("x"));
  expect(x1).toBeGreaterThan(x0);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("a selected <text> can be resized + rotated with the transform box", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text x="20" y="50" font-size="14">hello</text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");
  await page.locator(".layerlist .row-btn").filter({ hasText: "hello" }).click();

  const t = page.locator("svg.canvas g.artwork text");
  const handles = page.locator("svg.canvas g.overlay rect.xf-handle");
  await expect(handles).toHaveCount(8);

  // Resize via the SE corner handle (index 4) → the text scales up (a transform matrix appears).
  const w0 = (await t.boundingBox())!.width;
  const se = (await handles.nth(4).boundingBox())!;
  await page.mouse.move(se.x + se.width / 2, se.y + se.height / 2);
  await page.mouse.down();
  await page.mouse.move(se.x + 50, se.y + 30, { steps: 6 });
  await page.mouse.up();
  await expect(t).toHaveAttribute("transform", /matrix/);
  expect((await t.boundingBox())!.width).toBeGreaterThan(w0);

  // Rotate via the knob → the transform gains rotation (off-diagonal matrix terms ≠ 0).
  const knob = (await page.locator("svg.canvas g.overlay circle.rotate-knob").boundingBox())!;
  await page.mouse.move(knob.x + knob.width / 2, knob.y + knob.height / 2);
  await page.mouse.down();
  await page.mouse.move(knob.x + 40, knob.y + 25, { steps: 6 });
  await page.mouse.up();
  const tr = (await t.getAttribute("transform")) ?? "";
  const m = tr.match(/matrix\(([^)]+)\)/)?.[1].split(/[\s,]+/).map(Number) ?? [];
  expect(Math.abs(m[1] ?? 0) + Math.abs(m[2] ?? 0)).toBeGreaterThan(0.01);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("a source-defined gradient fill is editable in place (adopts on edit, keeps its id)", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><linearGradient id="a"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs><rect x="10" y="10" width="80" height="80" fill="url(#a)"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");
  await page.locator(".layerlist .row-btn").first().click();

  // A plain objectBoundingBox gradient is now editable in place: the kind reads "linear" and the
  // editable bar shows its two stops (not a read-only preview of the raw url string).
  const fill = page.locator(".paint").filter({ hasText: "fill" });
  await expect(page.getByLabel("fill kind")).toHaveValue("linear");
  const bar = fill.locator(".bar.editable");
  await expect(bar).toBeVisible();
  await expect(fill.locator(".marker")).toHaveCount(2);

  // Editing it (click the bar to add a stop) adopts it into the model — same id, so the fill
  // still references url(#a); the export defines #a exactly once (source def deduped).
  const bb = (await bar.boundingBox())!;
  await page.mouse.click(bb.x + bb.width / 2, bb.y + bb.height / 2);
  await expect(fill.locator(".marker")).toHaveCount(3);
  await expect(page.locator('svg.canvas g.artwork path[fill="url(#a)"]')).toHaveCount(1);

  await page.getByRole("button", { name: "source" }).click();
  const src = await page.locator(".sourceview textarea").inputValue();
  expect(src.match(/id="a"/g)?.length).toBe(1);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("nested groups: group a selection into a <g>, then ungroup", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="0" y="0" width="20" height="20" fill="#f00"/><rect x="40" y="0" width="20" height="20" fill="#00f"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Two shape rows in the panel (the imported rects), no group yet.
  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(2);
  await expect(page.locator(".layerlist .grouphead")).toHaveCount(0);

  // Select both, group → a nested <g> group header appears (rows stay, now nested).
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "group selection" }).click();
  await expect(page.locator(".layerlist .grouphead")).toHaveCount(1);
  await expect(rows).toHaveCount(2);

  // Export carries the nested <g> wrapping both rects.
  await page.getByRole("button", { name: "source" }).click();
  const src1 = await page.locator(".sourceview textarea").inputValue();
  expect(src1).toContain("<g id=\"group 1\">");
  expect(src1.match(/<rect/g)?.length).toBe(2);

  // Ungroup via the group header's context menu → the group dissolves.
  await page.locator(".layerlist .grouphead").click({ button: "right" });
  await page.getByRole("menuitem", { name: "ungroup" }).click();
  await expect(page.locator(".layerlist .grouphead")).toHaveCount(0);
  const src2 = await page.locator(".sourceview textarea").inputValue();
  expect(src2).not.toContain("<g id=\"group 1\"");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("structural edits (a group) survive a session reload", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="0" y="0" width="20" height="20" fill="#f00"/><rect x="40" y="0" width="20" height="20" fill="#00f"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(2);
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "group selection" }).click();
  await expect(page.locator(".layerlist .grouphead")).toHaveCount(1);

  // Let the debounced persist flush, then reload — the group must come back (tree persisted).
  await page.waitForTimeout(500);
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await expect(page.locator(".layerlist .grouphead")).toHaveCount(1);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("clicking a grouped shape selects the whole group; double-click drills in", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="0" y="0" width="30" height="30" fill="#f00"/><rect x="40" y="0" width="30" height="30" fill="#00f"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Group the two rects.
  const rows = page.locator(".layerlist .row-btn");
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "group selection" }).click();
  await expect(page.locator(".layerlist .grouphead")).toHaveCount(1);

  // Deselect, then a single click on a member selects the WHOLE group (→ the arrange/multi panel).
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("no canvas");
  await page.mouse.click(box.x + box.width * 0.9, box.y + box.height * 0.9);
  await page.locator("svg.canvas g.artwork path").first().click();
  await expect(page.getByRole("heading", { name: /arrange/ })).toBeVisible();

  // Double-click a member drills in → node editing that shape (anchors appear, group box gone).
  await page.locator("svg.canvas g.artwork path").first().dblclick();
  await expect(page.locator("svg.canvas g.overlay .anchor").first()).toBeVisible();
  await expect(page.getByRole("heading", { name: /arrange/ })).toHaveCount(0);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("clicking a filled shape's interior selects it (fill hit-test)", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });

  // Paste a filled square whose middle sits at the viewBox centre (→ the canvas centre).
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M20 20 H80 V80 H20 Z" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await expect(page.locator("svg.canvas g.artwork path")).toBeAttached();

  // Click the canvas centre (= doc 50,50, inside the fill but far from every edge/anchor).
  await page.keyboard.press("v");
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);

  // The whole path is now object-selected → the transform box is drawn in the overlay.
  await expect(page.locator("svg.canvas g.overlay polygon.sel-box")).toBeAttached();

  // Rotate via the knob above the box → the path geometry (its `d`) changes.
  const beforeD = await page.locator("svg.canvas g.artwork path").getAttribute("d");
  const knob = await page.locator("svg.canvas g.overlay .rotate-knob").boundingBox();
  if (!knob) throw new Error("no rotate knob");
  await page.mouse.move(knob.x + knob.width / 2, knob.y + knob.height / 2);
  await page.mouse.down();
  await page.mouse.move(knob.x + 45, knob.y + 30);
  await page.mouse.up();
  await expect(page.locator("svg.canvas g.artwork path")).not.toHaveAttribute("d", beforeD ?? "");

  // Give it a stroke first — cap/width/dash are inert (disabled) without one.
  await page.getByLabel("stroke on").click();

  // Styling round-trips through the core: set the stroke cap and see it on the element.
  await page
    .locator(".segrow")
    .filter({ hasText: "cap" })
    .getByRole("button", { name: "round" })
    .click();
  await expect(page.locator("svg.canvas g.artwork path")).toHaveAttribute("stroke-linecap", "round");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("create a component from a selection, then stamp an instance", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  // The "create component" flow prompts for a name.
  page.on("dialog", (d) => void d.accept("die"));

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="0" y="0" width="20" height="20" fill="#f00"/><rect x="40" y="0" width="20" height="20" fill="#00f"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Select both rects, then create a component from them.
  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(2);
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "create component from selection" }).click();

  // The def group + a single instance exist; both rects moved into the definition.
  await page.getByRole("button", { name: "source" }).click();
  const src1 = await page.locator(".sourceview textarea").inputValue();
  expect(src1).toContain("<defs");
  expect(src1).toContain('<g id="die"');
  expect(src1.match(/<use[^>]*href="#die"/g)?.length).toBe(1);
  expect(src1.match(/<rect/g)?.length).toBe(2);
  // A component row is listed in the panel.
  await expect(page.locator(".complist .comprow")).toHaveCount(1);

  // Stamp a second instance → two <use href="#die">.
  await page.locator(".comprow .stamp").click();
  const src2 = await page.locator(".sourceview textarea").inputValue();
  expect(src2.match(/<use[^>]*href="#die"/g)?.length).toBe(2);
  // Still one definition + two rects (the instance references the def, doesn't copy it).
  expect(src2.match(/<g id="die"/g)?.length).toBe(1);
  expect(src2.match(/<rect/g)?.length).toBe(2);

  // Expand the component + select a definition part — editing it (fill/geometry) propagates to
  // every instance (they're <use> of the one definition).
  await page.locator(".comprow .disclosure").first().click();
  const parts = page.locator(".partlist .part-btn");
  await expect(parts).toHaveCount(2);
  await parts.nth(0).click();
  await expect(parts.nth(0)).toHaveClass(/active/);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("detach bakes one instance into shapes; deleting a component cascades to its instances", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("dialog", (d) => void d.accept()); // the delete confirm()

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  // A component (die) already referenced by two <use> instances.
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><g id="die"><rect x="0" y="0" width="9" height="9" fill="#f00"/></g></defs><use href="#die"/><use href="#die" x="40" y="10"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // One component, two instances. The def's rect projects once; the two <use> don't emit rects.
  await expect(page.locator(".complist .comprow")).toHaveCount(1);
  await expect(page.locator(".comprow .meta")).toContainText("2×");
  await page.getByRole("button", { name: "source" }).click();
  const src0 = await page.locator(".sourceview textarea").inputValue();
  expect(src0.match(/<use/g)?.length).toBe(2);
  expect(src0.match(/<rect/g)?.length).toBe(1);

  // Select a <use> instance (a leaf layer row) → the element section offers "detach instance".
  await page.locator(".layerlist .pathrow .row-btn").first().click();
  await page.getByRole("button", { name: "detach instance" }).click();

  // The baked copy is independent shapes (rect count 1→2); one <use> remains; the def is untouched.
  const src1 = await page.locator(".sourceview textarea").inputValue();
  expect(src1.match(/<use/g)?.length).toBe(1);
  expect(src1.match(/<rect/g)?.length).toBe(2);
  expect(src1).toContain('<g id="die"');

  // Delete the component → its def AND the remaining instance cascade away; the baked group stays.
  await page.locator(".comprow .del").click();
  await expect(page.locator(".complist .comprow")).toHaveCount(0);
  const src2 = await page.locator(".sourceview textarea").inputValue();
  expect(src2).not.toContain("<use");
  expect(src2).not.toContain('id="die"');
  expect(src2.match(/<rect/g)?.length).toBe(1); // only the baked copy survives

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("grouping a non-adjacent multi-selection then deleting removes the right shapes", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  // Three rects; the middle one is green — grouping the two OUTER ones reindexes the flat path
  // list, so a stale numeric selection would delete a mis-indexed neighbour (the green one).
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 300 100"><path d="M10 10 H40 V40 H10 Z" fill="#f00"/><path d="M60 10 H90 V40 H60 Z" fill="#0f0"/><path d="M110 10 H140 V40 H110 Z" fill="#00f"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const artwork = page.locator("svg.canvas g.artwork path");
  await expect(artwork).toHaveCount(3);
  // LAYERS rows are reversed (top-of-stack first), so the middle row is always the middle shape
  // (green). Select the two outer rows (skipping green), group, delete.
  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(3);
  await rows.nth(0).click();
  await rows.nth(2).click({ modifiers: ["Shift"] });
  await page.keyboard.press("Meta+g");
  await page.keyboard.press("Delete");

  // The two grouped (outer) rects are gone; the untouched green survivor proves the selection was
  // re-derived by uid after the group reindexed the paths — not left pointing at a neighbour.
  await expect(artwork).toHaveCount(1);
  await expect(artwork.first()).toHaveAttribute("fill", "#0f0");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("a group can be renamed from the layers panel", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><path d="M10 10 H40 V40 H10 Z" fill="#f00"/><path d="M60 10 H90 V40 H60 Z" fill="#00f"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const rows = page.locator(".layerlist .row-btn");
  await expect(rows).toHaveCount(2);
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });
  await page.keyboard.press("Meta+g");

  // Double-click the group header name → inline rename → the header shows the new name.
  const gname = page.locator(".grouphead .lname");
  await expect(gname).toBeVisible();
  await gname.dblclick();
  const input = page.locator(".grouphead input.rename");
  await input.fill("my-group");
  await input.press("Enter");
  await expect(page.locator(".grouphead .lname")).toHaveText("my-group");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("pen closes the loop on a click at the start node, even with snap off", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  // Snap-to-points OFF so closing can't lean on the global nearest-anchor snap — it must recognise
  // the subpath's own start node directly. This is the corner-close bug: with snap off (or when a
  // neighbouring anchor is nearer), clicking the start used to append a stray node and leave the
  // path open, cutting a seam across the fill.
  await page.addInitScript(() => {
    try {
      localStorage.setItem(
        "nib:prefs",
        JSON.stringify({
          snapEnabled: false,
          snapThresholdPx: 12,
          gridEnabled: false,
          gridSize: 10,
          guidesEnabled: true,
        }),
      );
    } catch {
      /* non-fatal */
    }
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "new", exact: true }).click();

  await page.keyboard.press("p"); // pen tool
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  const at = (fx: number, fy: number) => ({ x: box.x + box.width * fx, y: box.y + box.height * fy });
  const start = at(0.4, 0.32); // node 0 — a corner (plain click, no handles)

  await page.mouse.click(start.x, start.y);
  const b = at(0.62, 0.34);
  await page.mouse.click(b.x, b.y);
  const c = at(0.52, 0.55);
  await page.mouse.click(c.x, c.y);
  await page.mouse.click(start.x, start.y); // click the start again → close the loop

  // A closed subpath serialises with a trailing Z; the bug left it open (an appended stray node, no Z).
  const path = page.locator("svg.canvas g.artwork path").first();
  await expect(path).toHaveAttribute("d", /[zZ]\s*$/);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("double-click a node toggles it between corner and smooth", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 140"><path d="M 60 40 L 140 40 L 100 100 Z" fill="#8888aa"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await expect(page.locator("svg.canvas g.artwork path")).toBeAttached();

  // Enter node mode by double-clicking the shape → its 3 corners show as SQUARE anchors.
  await page.locator("svg.canvas g.artwork path").first().dblclick();
  await expect(page.locator("svg.canvas rect.anchor")).toHaveCount(3);
  await expect(page.locator("svg.canvas circle.anchor")).toHaveCount(0);

  // Double-click one anchor → it becomes SMOOTH (a circle; tangents synthesized).
  const bb = await page.locator("svg.canvas rect.anchor").first().boundingBox();
  if (!bb) throw new Error("no anchor bbox");
  const cx = bb.x + bb.width / 2;
  const cy = bb.y + bb.height / 2;
  await page.mouse.dblclick(cx, cy);
  await expect(page.locator("svg.canvas circle.anchor")).toHaveCount(1);
  await expect(page.locator("svg.canvas rect.anchor")).toHaveCount(2);

  // Double-click it again → back to a CORNER (square), tangents stripped.
  await page.mouse.dblclick(cx, cy);
  await expect(page.locator("svg.canvas circle.anchor")).toHaveCount(0);
  await expect(page.locator("svg.canvas rect.anchor")).toHaveCount(3);

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

// A <text> has no anchors to node-edit, so double-click means "edit the words": an input opens
// over the label, seeded + selected, committing on Enter as a single undo step.
// A label carries its typeface as plain attributes, so the inspector edits them as text with
// suggestions rather than as a closed picker — a family this machine lacks is still the right
// answer for a file that opens elsewhere. The LAYERS row for a label says what the label says.
test("a label's typeface is editable and its layer row reads its words", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });

  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text x="20" y="50" font-size="10" fill="#000">Hello</text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");

  // The row names itself from the label's own words, and stands in a "T" for the thumbnail a
  // label has no geometry to draw.
  const row = page.locator(".layerlist .row-btn").first();
  await expect(row).toHaveText("Hello");
  await expect(page.locator(".layerlist .thumb.glyph")).toHaveText("T");

  await row.click();
  const family = page.locator('input[list="nib-font-families"]');
  await expect(family).toHaveValue("");

  await family.fill("serif");
  await family.press("Enter");
  const label = page.locator("svg.canvas text[data-uid]");
  await expect(label).toHaveAttribute("font-family", "serif");

  // Slant is a toggle, and it lights while it's on.
  const italic = page.getByRole("button", { name: "italic", exact: true });
  await italic.click();
  await expect(label).toHaveAttribute("font-style", "italic");
  await expect(italic).toHaveClass(/\bon\b/);
  await italic.click();
  await expect(label).not.toHaveAttribute("font-style", "italic");

  // Each is one undo step — a typeface change rides the same history as any other edit.
  await page.keyboard.press("Meta+z");
  await expect(label).toHaveAttribute("font-style", "italic");
});

// The paint field's keyword picker. `none` used to sit twice in the fill block — once as a mode
// chip, once as a `—` button on the row below — which reads as a mistake even though both worked.
// The button is now a picker for the values a swatch can't reach, and it leaves `none` to the chip.
// A paint answers two questions — *is there one* and *what kind* — and they used to share one row
// of four chips, which is how "no fill" ended up with two controls and the panel ended up busy.
// A switch owns the first, a short list owns the second, and only the chosen kind's controls show.
test("a paint is a switch plus a kind, and off collapses the block", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });

  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="20" y="20" width="60" height="40" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");
  await page.locator(".layerlist .row-btn").first().click();

  const shape = page.locator("svg.canvas g.artwork path");
  const fillOn = page.getByLabel("fill on");
  const fillKind = page.getByLabel("fill kind");

  // On, and a colour. The old four-chip row is gone entirely.
  await expect(fillOn).toHaveAttribute("aria-checked", "true");
  await expect(fillKind).toHaveValue("color");
  await expect(page.locator(".paint").filter({ hasText: "fill" }).locator(".pmode")).toHaveCount(0);

  // `currentColor` is a KIND, not a separate keyword picker beside the field.
  await fillKind.selectOption("currentColor");
  await expect(shape).toHaveAttribute("fill", "currentColor");
  await fillKind.selectOption("color");
  await expect(shape).toHaveAttribute("fill", "#3b82f6"); // and the colour came back

  // The switch owns "none" — one control, and turning it off takes the whole block with it.
  await fillOn.click();
  await expect(shape).toHaveAttribute("fill", "none");
  await expect(fillKind).toHaveCount(0);
  await expect(page.getByLabel("fill eyedropper")).toHaveCount(0);

  // Back on restores the colour rather than dumping you at black.
  await fillOn.click();
  await expect(shape).toHaveAttribute("fill", "#3b82f6");
});

test("double-click a text label edits it in place", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });

  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text x="20" y="50" font-size="10" fill="#000">Hello</text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  const label = page.locator("svg.canvas text[data-uid]");
  await expect(label).toHaveText("Hello");

  await label.dblclick();
  const input = page.locator("input.text-edit");
  await expect(input).toBeVisible();
  await expect(input).toHaveValue("Hello");

  // The field opens select-all, so typing replaces (rename-field behaviour).
  await page.keyboard.type("Goodbye");
  await page.keyboard.press("Enter");
  await expect(input).toHaveCount(0);
  await expect(label).toHaveText("Goodbye");

  // Escape abandons an edit rather than committing it.
  await label.dblclick();
  await page.keyboard.type("Discarded");
  await page.keyboard.press("Escape");
  await expect(page.locator("input.text-edit")).toHaveCount(0);
  await expect(label).toHaveText("Goodbye");

  // One undo step per commit.
  await page.keyboard.press("Meta+z");
  await expect(label).toHaveText("Hello");
});

// Regression: an element gesture writes into the element's PARENT coordinate space, so a
// document-space delta over-translated anything nested under a scaled <g> — inside scale(8) the
// label ran eight times too fast and left the frame within the first few pixels of a drag.
test("dragging a text nested in a scaled group tracks the cursor 1:1", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });

  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><g transform="scale(8)"><text x="5" y="6" font-size="2" fill="#000">Hi</text></g></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  const label = page.locator("svg.canvas text[data-uid]");
  await expect(label).toBeAttached();

  const before = await label.boundingBox();
  if (!before) throw new Error("no label bbox");

  // Select, then drag 24 screen px to the right.
  const gx = before.x + before.width / 2;
  const gy = before.y + before.height / 2;
  await page.mouse.click(gx, gy);
  await page.mouse.move(gx, gy);
  await page.mouse.down();
  await page.mouse.move(gx + 12, gy, { steps: 4 });
  await page.mouse.move(gx + 24, gy, { steps: 4 });
  await page.mouse.up();

  const after = await label.boundingBox();
  if (!after) throw new Error("no label bbox after drag");
  // 1:1 with the cursor, within rounding. Pre-fix this moved ~192px.
  expect(Math.abs(after.x - before.x - 24)).toBeLessThanOrEqual(2);
  expect(Math.abs(after.y - before.y)).toBeLessThanOrEqual(2);
});

// Regression (engine-specific, hence @cross): the start matrix of an element gesture is snapshotted
// rather than held as the live `consolidate().matrix` view — Firefox mutates that object as the
// transform attribute is written, so composing onto it made each pointermove build on the previous
// frame. A move accelerated away from the cursor and a rotate spun far past the knob.
test(
  "moving and rotating an already-transformed text tracks the cursor",
  { tag: "@cross" },
  async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
      timeout: 30_000,
    });

    await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
    await page
      .locator("textarea")
      .fill(
        `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text transform="translate(20 40)" font-size="10" fill="#000">Hello</text></svg>`,
      );
    await page.keyboard.press("Meta+Enter");
    await page.keyboard.press("v");
    const label = page.locator("svg.canvas text[data-uid]");
    await expect(label).toBeAttached();

    // Client rects read in-page, not via Playwright's boundingBox: the mouse drives client
    // coordinates and the editor measures the element the same way, and the two disagree in WebKit.
    const rectOf = (l: typeof label) => l.evaluate((el) => el.getBoundingClientRect().toJSON());

    const before = await rectOf(label);
    // Selecting a label needs the pointer on painted ink, and a <text>'s client rect is roomier
    // than its glyphs (by different amounts per engine), so probe for a point that actually hits.
    const ink = await page.evaluate((r) => {
      for (let fy = 0.2; fy < 0.9; fy += 0.1)
        for (let fx = 0.05; fx < 0.9; fx += 0.05) {
          const x = r.x + r.width * fx;
          const y = r.y + r.height * fy;
          if (document.elementFromPoint(x, y)?.closest("text[data-uid]")) return { x, y };
        }
      return null;
    }, before);
    if (!ink) throw new Error("no point on the label's glyphs");
    const gx = ink.x;
    const gy = ink.y;
    await page.mouse.click(gx, gy);
    await expect(page.locator("svg.canvas g.overlay polygon.sel-box")).toBeAttached();
    // Past the double-click interval, else the drag's mousedown reads as a double-click and opens
    // the inline label editor instead.
    await page.waitForTimeout(600);
    await page.mouse.move(gx, gy);
    await page.mouse.down();
    await page.mouse.move(gx + 20, gy, { steps: 4 });
    await page.mouse.move(gx + 40, gy, { steps: 4 });
    await page.mouse.move(gx + 60, gy, { steps: 4 });
    await page.mouse.up();

    const moved = await rectOf(label);
    // 1:1 with the cursor: the third step compounded to ~120px before the fix.
    expect(Math.abs(moved.x - before.x - 60)).toBeLessThanOrEqual(3);
    expect(Math.abs(moved.y - before.y)).toBeLessThanOrEqual(3);

    // Rotate: the label turns by exactly the angle the knob was dragged through, around the box
    // centre. Pre-fix each pointermove composed onto the previous frame, so the angle compounded.
    const cx = moved.x + moved.width / 2;
    const cy = moved.y + moved.height / 2;
    const knob = await rectOf(page.locator("svg.canvas g.overlay circle.rotate-knob"));
    const kx = knob.x + knob.width / 2;
    const ky = knob.y + knob.height / 2;
    await page.mouse.move(kx, ky);
    await page.mouse.down();
    await page.mouse.move(kx + 10, ky + 4, { steps: 3 });
    await page.mouse.move(kx + 20, ky + 12, { steps: 3 });
    await page.mouse.move(kx + 30, ky + 20, { steps: 3 });
    await page.mouse.up();

    const deg = (rad: number) => (rad * 180) / Math.PI;
    const dragged =
      deg(Math.atan2(ky + 20 - cy, kx + 30 - cx)) - deg(Math.atan2(ky - cy, kx - cx));
    const matrix = await label.evaluate((el) => el.getAttribute("transform") ?? "");
    const [, a, b] = matrix.match(/matrix\(\s*([-\d.]+)[\s,]+([-\d.]+)/) ?? [];
    const turned = deg(Math.atan2(Number(b), Number(a)));
    expect(Math.abs(turned - dragged)).toBeLessThanOrEqual(2);
  },
);

// A real outline font off the host, for the text-outlining test. Font files can't be checked into
// the repo (licences, megabytes), so the test uses whatever the machine has and skips where a
// runner has none.
function systemFontPath(): string | null {
  const candidates = [
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
  ];
  return candidates.find((p) => existsSync(p)) ?? null;
}

// Text is the one thing in a drawing nib can't reshape — a label carries no anchor geometry, so it
// can't be node-edited or boolean'd, and it renders wrong wherever the font is missing. Converting
// it to outlines is the escape hatch: shaped glyphs (rustybuzz, in the core) become an ordinary
// editable path.
test(
  "converts a text label into an editable outlined path",
  { tag: "@cross" },
  async ({ page }) => {
    const fontPath = systemFontPath();
    test.skip(!fontPath, "no system font on this machine to outline with");

    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));

    // Force the file-picker path: where the Local Font Access API exists it would answer first, and
    // this test is about the fallback every other browser takes anyway.
    await page.addInitScript(() => {
      delete (window as unknown as { queryLocalFonts?: unknown }).queryLocalFonts;
    });

    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
      timeout: 30_000,
    });

    await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
    await page
      .locator("textarea")
      .fill(
        `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text x="10" y="60" font-size="40" fill="#111111">Hi</text></svg>`,
      );
    await page.keyboard.press("Meta+Enter");
    await page.keyboard.press("v");
    await expect(page.locator("svg.canvas g.artwork text")).toHaveCount(1);
    await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(0);

    // Select the label on canvas — on its glyphs, since the client rect is roomier than the ink.
    const label = page.locator("svg.canvas text[data-uid]");
    const box = await label.evaluate((el) => el.getBoundingClientRect().toJSON());
    const ink = await page.evaluate((r) => {
      for (let fy = 0.2; fy < 0.9; fy += 0.1)
        for (let fx = 0.05; fx < 0.9; fx += 0.05) {
          const x = r.x + r.width * fx;
          const y = r.y + r.height * fy;
          if (document.elementFromPoint(x, y)?.closest("text[data-uid]")) return { x, y };
        }
      return null;
    }, box);
    if (!ink) throw new Error("no point on the glyphs");
    await page.mouse.click(ink.x, ink.y);

    const [chooser] = await Promise.all([
      page.waitForEvent("filechooser"),
      page.getByRole("button", { name: "convert to outlines" }).click(),
    ]);
    await chooser.setFiles(fontPath as string);

    // The label is geometry now: a path in the artwork, no <text> left.
    await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(1);
    await expect(page.locator("svg.canvas g.artwork text")).toHaveCount(0);
    const d = await page.locator("svg.canvas g.artwork path").getAttribute("d");
    expect(d?.length ?? 0).toBeGreaterThan(50);

    // …and an ordinary editable shape: double-click drills into node editing. (The glyphs sit inside
    // the label's old client rect — a `<text>` box includes side bearing — so re-probe for ink.)
    const pathBox = await page
      .locator("svg.canvas g.artwork path")
      .evaluate((el) => el.getBoundingClientRect().toJSON());
    const glyph = await page.evaluate((r) => {
      for (let fy = 0.1; fy < 0.95; fy += 0.05)
        for (let fx = 0.02; fx < 0.95; fx += 0.02) {
          const x = r.x + r.width * fx;
          const y = r.y + r.height * fy;
          if (document.elementFromPoint(x, y)?.closest("path[data-uid]")) return { x, y };
        }
      return null;
    }, pathBox);
    if (!glyph) throw new Error("no point on the outlined glyphs");
    await page.mouse.dblclick(glyph.x, glyph.y);
    await expect(page.locator("svg.canvas g.overlay .anchor").first()).toBeAttached();

    // Undo brings the words back — destructive, but not a one-way door.
    await page.keyboard.press("Meta+z");
    await expect(page.locator("svg.canvas g.artwork text")).toHaveCount(1);

    expect(errors, `page errors:\n${errors.join("\n")}`).toEqual([]);
  },
);

// Outlining a label whose family the machine doesn't have still works — some face answers — but
// the letterforms then aren't the ones the document described. That has to be said out loud, or
// the next person reading the drawing concludes nib mangled their text.
test("says which font a label was outlined with when it isn't the one it asked for", async ({
  page,
}) => {
  const fontPath = systemFontPath();
  test.skip(!fontPath, "no system font on this machine to outline with");

  await page.addInitScript(() => {
    delete (window as unknown as { queryLocalFonts?: unknown }).queryLocalFonts;
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });

  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text x="10" y="60" font-size="40" font-family="Nonexistent Brand Face" fill="#111111">Hi</text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const label = page.locator("svg.canvas text[data-uid]");
  const box = await label.evaluate((el) => el.getBoundingClientRect().toJSON());
  const ink = await page.evaluate((r) => {
    for (let fy = 0.2; fy < 0.9; fy += 0.1)
      for (let fx = 0.05; fx < 0.9; fx += 0.05) {
        const x = r.x + r.width * fx;
        const y = r.y + r.height * fy;
        if (document.elementFromPoint(x, y)?.closest("text[data-uid]")) return { x, y };
      }
    return null;
  }, box);
  if (!ink) throw new Error("no point on the glyphs");
  await page.mouse.click(ink.x, ink.y);

  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.getByRole("button", { name: "convert to outlines" }).click(),
  ]);
  await chooser.setFiles(fontPath as string);

  // It converted…
  await expect(page.locator("svg.canvas g.artwork path")).toHaveCount(1);
  // …and said so, naming the face it actually used. A notice, not an error.
  const notice = page.locator(".errbar.notice");
  await expect(notice).toContainText("outlined with");
  await expect(notice).toContainText("Nonexistent Brand Face");
  await expect(page.locator(".errbar[role='alert']")).toHaveCount(0);

  // It's dismissible, and stays dismissed.
  await notice.getByRole("button", { name: "dismiss notice" }).click();
  await expect(notice).toHaveCount(0);
});

// Multi-line text is how design tools export it — one `<tspan>` per line — so this is the shape
// real files bring in. It has to convert, and convert as the lines it is rather than as one
// collapsed run.
test("outlines a multi-line tspan label as separate lines", async ({ page }) => {
  const fontPath = systemFontPath();
  test.skip(!fontPath, "no system font on this machine to outline with");

  await page.addInitScript(() => {
    delete (window as unknown as { queryLocalFonts?: unknown }).queryLocalFonts;
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });

  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><text x="20" y="60" font-size="28" fill="#111111"><tspan x="20" y="60">first</tspan><tspan x="20" y="120">second</tspan></text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const label = page.locator("svg.canvas text[data-uid]");
  await expect(label).toBeAttached();
  const before = await label.evaluate((el) => el.getBoundingClientRect().toJSON());

  const ink = await page.evaluate((r) => {
    for (let fy = 0.1; fy < 0.95; fy += 0.05)
      for (let fx = 0.02; fx < 0.95; fx += 0.05) {
        const x = r.x + r.width * fx;
        const y = r.y + r.height * fy;
        if (document.elementFromPoint(x, y)?.closest("text[data-uid]")) return { x, y };
      }
    return null;
  }, before);
  if (!ink) throw new Error("no point on the glyphs");
  await page.mouse.click(ink.x, ink.y);

  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.getByRole("button", { name: "convert to outlines" }).click(),
  ]);
  await chooser.setFiles(fontPath as string);

  const path = page.locator("svg.canvas g.artwork path");
  await expect(path).toHaveCount(1);
  await expect(page.locator("svg.canvas g.artwork text")).toHaveCount(0);

  // Both lines are in there: the outlines span the 60 user units between the two baselines, which
  // one collapsed run never would.
  const after = await path.evaluate((el) => el.getBoundingClientRect().toJSON());
  expect(after.height).toBeGreaterThan(before.height * 0.6);
  const d = (await path.getAttribute("d")) ?? "";
  expect(d.length).toBeGreaterThan(100);
});

// A filter's surface is sized from the object in *device* space, so a drop shadow on a shape zoomed
// far in becomes a several-hundred-megapixel blur. WebKit re-rasterizes that every frame and
// panning collapses to a few fps (measured: 3.6 vs 60 on the same document in Chromium). Filters
// therefore go quiet for the duration of a gesture and come back with the next still frame.
//
// Frame rates are too machine-dependent to assert, so this pins the mechanism instead.
test(
  "filters go quiet while the view moves, and come back when it stops",
  { tag: "@cross" },
  async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
      timeout: 30_000,
    });

    await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
    await page
      .locator("textarea")
      .fill(
        `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><filter id="sh"><feDropShadow dx="1" dy="1" stdDeviation="1.5"/></filter></defs><rect x="20" y="20" width="40" height="40" fill="#3b82f6" filter="url(#sh)"/></svg>`,
      );
    await page.keyboard.press("Meta+Enter");

    const scene = page.locator("svg.canvas g.scene");
    const shape = page.locator("svg.canvas g.artwork [filter]");
    await expect(shape).toBeAttached();

    // At rest the document paints as authored.
    await expect(scene).not.toHaveClass(/interacting/);
    expect(await shape.evaluate((el) => getComputedStyle(el).filter)).toContain("#sh");

    // Panning by wheel is a gesture even though it never reaches the gesture machine.
    const box = (await page.locator("svg.canvas").boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.wheel(0, 60);
    await expect(scene).toHaveClass(/interacting/);
    expect(await shape.evaluate((el) => getComputedStyle(el).filter)).toBe("none");

    // …and the accurate frame returns once the wheel stops.
    await expect(scene).not.toHaveClass(/interacting/, { timeout: 2000 });
    expect(await shape.evaluate((el) => getComputedStyle(el).filter)).toContain("#sh");
  },
);

// The rotate tool exists for the one turn the select tool's box can't do: swinging a shape around
// a point that isn't its own centre. So the test is exactly that difference — a rotation about a
// placed pivot *moves* the shape, where a centre rotation would leave it where it is.
test("rotates about a pivot you place, not the shape's centre", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });

  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="40" y="40" width="20" height="20" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");

  const shape = page.locator("svg.canvas g.artwork path");
  await expect(shape).toBeAttached();

  // Select it with the select tool, then switch to rotate.
  await page.keyboard.press("v");
  const canvas = (await page.locator("svg.canvas").boundingBox())!;
  const centre = await shape.evaluate((el) => {
    const b = (el as SVGGraphicsElement).getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
  });
  await page.mouse.click(centre.x, centre.y);
  await expect(page.locator("svg.canvas g.overlay polygon.sel-box")).toBeAttached();

  await page.keyboard.press("e");
  // The pivot shows at the selection's centre until it's moved.
  const pivot = page.locator("svg.canvas g.overlay .pivot-dot");
  await expect(pivot).toBeAttached();
  const atCentre = await pivot.evaluate((el) => ({
    x: Number(el.getAttribute("cx")),
    y: Number(el.getAttribute("cy")),
  }));
  expect(Math.abs(atCentre.x + canvas.x - centre.x)).toBeLessThan(3);

  // Click well away from the shape to place the pivot there.
  const placed = { x: centre.x - 120, y: centre.y };
  await page.mouse.click(placed.x, placed.y);
  const moved = await pivot.evaluate((el) => Number(el.getAttribute("cx")));
  expect(Math.abs(moved + canvas.x - placed.x)).toBeLessThan(3);

  const before = await shape.evaluate((el) => (el as SVGGraphicsElement).getBoundingClientRect().x);

  // Drag a half-turn about that pivot: grab a point right of the pivot and swing it to the left.
  await page.mouse.move(centre.x, centre.y);
  await page.mouse.down();
  await page.mouse.move(placed.x, placed.y - 100, { steps: 8 });
  await page.mouse.move(placed.x - 120, placed.y + 1, { steps: 8 });
  await page.mouse.up();

  // Half a turn about a pivot to the shape's left lands it on the pivot's far side — a centre
  // rotation of a square would have left the box exactly where it was.
  const after = await shape.evaluate((el) => (el as SVGGraphicsElement).getBoundingClientRect().x);
  expect(after).toBeLessThan(before - 100);

  // And it's one undo step, not one per frame.
  await page.keyboard.press("Meta+z");
  const undone = await shape.evaluate((el) => (el as SVGGraphicsElement).getBoundingClientRect().x);
  expect(Math.abs(undone - before)).toBeLessThan(3);
});

// The selection box turns with the shape and *stays* turned, the way Pixelmator's does. Its tilt
// is stored (`PathElement.boxAngle`) because the geometry isn't — a rotation bakes into the
// anchors — so without it the box could only ever be the axis-aligned bounds of the result: it
// breathed from wide to tall mid-swing while the handles sat still (which reads as a resize), and
// snapped upright on release, leaving no way to resize a turned shape along its own edges.
test("the selection box turns with the shape, and stays turned", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      // Deliberately wide and short: a square would hide the bug this pins.
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="30" y="35" width="40" height="20" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  const shapeEl = page.locator("svg.canvas g.artwork path");
  const shape = (await shapeEl.boundingBox())!;
  const centre = { x: shape.x + shape.width / 2, y: shape.y + shape.height / 2 };
  await page.mouse.click(centre.x, centre.y);

  const box = page.locator("svg.canvas g.overlay polygon.sel-box");

  const rest = await boxGeom(box);
  expect(Math.abs(rest.deg)).toBeLessThan(0.5); // upright, as authored
  expect(rest.w).toBeGreaterThan(rest.h); // and wide

  // Grab the knob and swing about a quarter turn, holding the drag open.
  const knob = await page
    .locator("svg.canvas g.overlay circle.rotate-knob")
    .evaluate((el) => {
      const b = (el as SVGGraphicsElement).getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    });
  await page.mouse.move(knob.x, knob.y);
  await page.mouse.down();
  await page.mouse.move(centre.x + 140, centre.y + 60, { steps: 6 });

  const spinning = await boxGeom(box);
  // The box is the one the drag started with, turned — same proportions, real rotation.
  expect(spinning.aspect).toBeCloseTo(rest.aspect, 1);
  expect(Math.abs(spinning.deg)).toBeGreaterThan(20);

  await page.mouse.up();

  // Released, the box keeps the angle it was turned to and still hugs the shape — it is not the
  // axis-aligned bounds of the result, which for a wide shape swung 113° would be far squarer.
  const settled = await boxGeom(box);
  expect(settled.deg).toBeCloseTo(spinning.deg, 0);
  expect(settled.aspect).toBeCloseTo(rest.aspect, 1);

  // And the handles came with it, so a resize pulls along the shape's own edges. Dragging the east
  // handle of a box turned off-axis has to move the shape's document-space bounds in *both*
  // directions; a scale along the document's x could only widen it.
  const bboxOf = async () =>
    await shapeEl.evaluate((el) => {
      const b = (el as SVGGraphicsElement).getBoundingClientRect();
      return { w: b.width, h: b.height };
    });
  const beforeResize = await bboxOf();
  const east = await page
    .locator("svg.canvas g.overlay rect.xf-handle")
    .nth(3)
    .evaluate((el) => {
      const b = (el as SVGGraphicsElement).getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    });
  await page.mouse.move(east.x, east.y);
  await page.mouse.down();
  await page.mouse.move(east.x + 60, east.y + 60, { steps: 5 });
  await page.mouse.up();
  const afterResize = await bboxOf();
  expect(Math.abs(afterResize.w - beforeResize.w)).toBeGreaterThan(5);
  expect(Math.abs(afterResize.h - beforeResize.h)).toBeGreaterThan(5);

  // The box is still at the same angle: a resize doesn't re-orient it.
  expect((await boxGeom(box)).deg).toBeCloseTo(settled.deg, 0);

  // The tilt rides history like any other edit: undo the resize, then the rotation, and the box
  // is upright again. (A stored angle that undo couldn't reach would leave the box describing a
  // shape that no longer exists.)
  await page.keyboard.press("Meta+z");
  await page.keyboard.press("Meta+z");
  await page.mouse.click(centre.x, centre.y); // undo restores the doc, not the selection
  await expect(box).toBeAttached();
  expect(Math.abs((await boxGeom(box)).deg)).toBeLessThan(0.5);
});

// A label reaches the same place by a different route: it has no anchor geometry, so its rotation
// lives as a matrix on the node and its box is measured from its own untransformed bbox mapped
// through its screen matrix. Same outcome as a shape's stored `boxAngle` — turned while turning,
// still turned after, handles on the object's own axes.
test("a label's box turns as it rotates, and stays turned", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text x="15" y="55" font-size="18" fill="#111">Rotate me</text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Select the label by clicking its ink (its client rect is roomier than its glyphs).
  const label = page.locator("svg.canvas text[data-uid]");
  const rect = await label.evaluate((el) => el.getBoundingClientRect().toJSON());
  const ink = await page.evaluate((r) => {
    for (let fy = 0.2; fy < 0.9; fy += 0.1)
      for (let fx = 0.05; fx < 0.9; fx += 0.05) {
        const x = r.x + r.width * fx;
        const y = r.y + r.height * fy;
        if (document.elementFromPoint(x, y)?.closest("text[data-uid]")) return { x, y };
      }
    return null;
  }, rect);
  if (!ink) throw new Error("no point on the label's glyphs");
  await page.mouse.click(ink.x, ink.y);

  const sel = page.locator("svg.canvas g.overlay polygon.sel-box");

  const rest = await boxGeom(sel);
  expect(Math.abs(rest.deg)).toBeLessThan(0.5);
  expect(rest.w).toBeGreaterThan(rest.h); // a line of text is wide

  const knob = await page
    .locator("svg.canvas g.overlay circle.rotate-knob")
    .evaluate((el) => {
      const b = (el as SVGGraphicsElement).getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    });
  const centre = { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
  await page.mouse.move(knob.x, knob.y);
  await page.mouse.down();
  await page.mouse.move(centre.x + 160, centre.y + 70, { steps: 6 });

  const spinning = await boxGeom(sel);
  expect(spinning.aspect).toBeCloseTo(rest.aspect, 1);
  expect(Math.abs(spinning.deg)).toBeGreaterThan(15);

  await page.mouse.up();

  // The label really rotated (a matrix on the node) — and the box held its angle rather than
  // collapsing to the axis-aligned bounds around it.
  expect(await label.getAttribute("transform")).toMatch(/^matrix\(/);
  const settled = await boxGeom(sel);
  expect(settled.deg).toBeCloseTo(spinning.deg, 0);
  expect(settled.aspect).toBeCloseTo(rest.aspect, 1);

  // And the handles came with it: the SE handle sits on the turned box's own corner, which is what
  // makes a resize after a rotation pull along the label's baseline instead of the parent's x.
  const corner = await sel.evaluate((el) => {
    const se = (el.getAttribute("points") ?? "").trim().split(/\s+/)[2];
    const [x, y] = se.split(",").map(Number);
    return { x, y };
  });
  const seHandle = await page
    .locator("svg.canvas g.overlay rect.xf-handle")
    .nth(4)
    .evaluate((el) => {
      const b = (el as SVGGraphicsElement).getBBox();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    });
  expect(Math.hypot(seHandle.x - corner.x, seHandle.y - corner.y)).toBeLessThan(1.5);
});

// The context-menu policy, which the interaction skill calls the 1.0 gate: one menu, every
// surface, and the browser's never appears except in a text field. A drawing surface invites
// right-click constantly — answering it with Back/Reload is the loudest inconsistency an app can
// ship, and "sometimes ours, sometimes theirs" is worse than either alone.
test("right-click answers with nib's menu everywhere but text fields", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });

  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect id="brick" x="30" y="30" width="40" height="40" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await expect(page.locator("svg.canvas g.artwork path")).toBeAttached();

  const menu = page.locator("[role='menu']");
  const shape = (await page.locator("svg.canvas g.artwork path").boundingBox())!;

  // 1. The canvas — the surface that used to give the browser's menu and nothing else.
  await page.mouse.click(shape.x + shape.width / 2, shape.y + shape.height / 2, {
    button: "right",
  });
  await expect(menu).toBeVisible();
  // It names its subject and carries the shape's verbs.
  await expect(menu).toContainText("brick");
  await expect(menu.getByRole("menuitem", { name: "duplicate" })).toBeVisible();
  // A verb that doesn't apply is greyed with a reason, not missing.
  const pasteStyle = menu.getByRole("menuitem", { name: "paste style" });
  await expect(pasteStyle).toBeDisabled();
  await expect(pasteStyle).toHaveAttribute("title", /copy a style first/);

  // Escape closes the menu — and only the menu, the outermost rung of the ladder.
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(page.locator("svg.canvas g.overlay polygon.sel-box")).toBeAttached();

  // 2. Empty canvas gets the document's verbs rather than nothing.
  await page.mouse.click(shape.x - 80, shape.y - 40, { button: "right" });
  await expect(menu).toContainText("canvas");
  await expect(menu.getByRole("menuitem", { name: "select all" })).toBeVisible();
  await page.keyboard.press("Escape");

  // 3. A LAYERS row answers with the same component.
  const row = page.locator("aside .layers li").first();
  await row.click({ button: "right" });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "rename" })).toBeVisible();
  await page.keyboard.press("Escape");

  // 4. Only one menu exists at a time, wherever it was opened from.
  await page.mouse.click(shape.x + shape.width / 2, shape.y + shape.height / 2, {
    button: "right",
  });
  await row.click({ button: "right" });
  await expect(menu).toHaveCount(1);
  await page.keyboard.press("Escape");

  // 5. Text fields keep the browser's menu: nib must not swallow paste and spellcheck. The
  //    handler runs at the window, so this asserts the exception survives every ancestor.
  const defaultPrevented = await page.evaluate(() => {
    const input = document.querySelector("aside input") as HTMLInputElement | null;
    if (!input) return null;
    const ev = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    input.dispatchEvent(ev);
    return ev.defaultPrevented;
  });
  expect(defaultPrevented).toBe(false);

  // …while anywhere else it is suppressed.
  const suppressed = await page.evaluate(() => {
    const ev = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    document.querySelector("svg.canvas")!.dispatchEvent(ev);
    return ev.defaultPrevented;
  });
  expect(suppressed).toBe(true);
});

// A node is a thing with verbs of its own — right-clicking one shouldn't answer about the shape
// it belongs to, any more than right-clicking a word should answer about the paragraph.
test("a node's own verbs live on its right-click", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });
  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M20 20 H80 V80 H20 Z" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Double-click into node editing, where anchors exist to be right-clicked. A filled shape and
  // its centre: this test is about the menu, not about hitting a hairline stroke.
  const shape = (await page.locator("svg.canvas g.artwork path").boundingBox())!;
  await page.mouse.dblclick(shape.x + shape.width / 2, shape.y + shape.height / 2);
  const anchor = page.locator("svg.canvas g.overlay .anchor").first();
  await expect(anchor).toBeAttached();

  const at = await anchor.evaluate((el) => {
    const b = (el as SVGGraphicsElement).getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
  });
  await page.mouse.click(at.x, at.y, { button: "right" });

  const menu = page.locator("[role='menu']");
  await expect(menu).toContainText("node");
  // The state it is already in is shown, greyed — not hidden, and not offered as a no-op.
  await expect(menu.getByRole("menuitem", { name: "corner" })).toBeDisabled();

  const before = await page.locator("svg.canvas g.artwork path").getAttribute("d");
  await menu.getByRole("menuitem", { name: "smooth" }).click();
  await expect(menu).toHaveCount(0);
  // Smoothing synthesises tangent handles, so the geometry really changed.
  expect(await page.locator("svg.canvas g.artwork path").getAttribute("d")).not.toBe(before);
});

// Undo lives in memory, so without a baseline a reload is the one gesture that makes every
// unsaved change permanent. Revert is the way back — and the one action here that asks first,
// because it's the one undo can't take back.
test("revert goes back to the saved file, after asking", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 30_000,
  });

  await page.locator("header").getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="30" y="30" width="20" height="20" fill="#3b82f6"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  const shape = page.locator("svg.canvas g.artwork path");
  await expect(shape).toBeAttached();
  const saved = await shape.getAttribute("d");

  // Change it: select and nudge.
  await page.keyboard.press("v");
  const box = (await shape.boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  for (let i = 0; i < 5; i++) await page.keyboard.press("ArrowRight");
  expect(await shape.getAttribute("d")).not.toBe(saved);

  // Revert asks before discarding, and cancelling keeps the edits.
  await page.keyboard.press("Meta+k");
  await page.locator(".palette .q").fill("revert");
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("revert to the saved file?");
  await dialog.getByRole("button", { name: "cancel" }).click();
  expect(await shape.getAttribute("d")).not.toBe(saved);

  // Confirming puts the file back as it was.
  await page.keyboard.press("Meta+k");
  await page.locator(".palette .q").fill("revert");
  await page.keyboard.press("Enter");
  await page.getByRole("dialog").getByRole("button", { name: "revert" }).click();
  await expect(page.locator("svg.canvas g.artwork path")).toHaveAttribute("d", saved ?? "");
});
