import { expect, test } from "@playwright/test";

// End-to-end proof that the Svelte UI drives the Rust/WASM engine correctly: boot → load →
// render → draw → undo, asserting no console/page errors throughout. This is the check that
// the document store's delegation to nib-core actually works in a browser (Phase A5).
test("boots the core, loads a sample, draws, and undoes without errors", async ({ page }) => {
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

test("draws a rectangle with the rect shape tool", async ({ page }) => {
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

test("New drawing creates a blank document from the top bar", async ({ page }) => {
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
  await page.locator("header").getByRole("button", { name: "New", exact: true }).click();
  await expect(page.locator("svg.canvas")).toBeVisible();
  await expect(page.locator("header .name")).toHaveText("untitled.svg");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});

test("the command palette opens and runs an action", async ({ page }) => {
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

test("double-click enters node editing — anchors appear only then", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-core-version", /\d+\.\d+\.\d+/, {
    timeout: 15_000,
  });
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await expect(page.locator("svg.canvas g.overlay rect.sel-box")).toBeAttached();
  await expect(page.locator("svg.canvas g.overlay .anchor")).toHaveCount(0);

  // Double-click enters node editing → the square's four anchors appear.
  await page.mouse.dblclick(cx, cy);
  await expect(page.locator("svg.canvas g.overlay .anchor")).toHaveCount(4);
  await expect(page.locator("svg.canvas g.overlay rect.sel-box")).toHaveCount(0);

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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await expect(page.locator("svg.canvas g.overlay rect.sel-box")).toBeAttached();
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
  await page.locator(".lhead .ghost-btn").click();
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
  await expect(page.locator("svg.canvas g.overlay rect.sel-box")).toBeAttached();

  // Fill → linear gradient: a <linearGradient> def appears and the shape references it.
  await page.locator(".paint").filter({ hasText: "fill" }).getByRole("button", { name: "linear" }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  const skewX = page.getByTitle("skew X (degrees)");
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text x="20" y="50" font-size="14" fill="#000000">hello</text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");

  // The <text> is a leaf row in the panel (not a shape/group). Selecting it opens the element
  // section (there are no shape rows — text isn't an editable path).
  const row = page.locator(".layerlist .row-btn").filter({ hasText: "text" });
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text x="20" y="50" font-size="14">hello</text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");

  // Select via the panel row (reliable), then the overlay draws a box around the text's DOM bbox.
  await page.locator(".layerlist .row-btn").filter({ hasText: "text" }).click();
  const selBox = page.locator("svg.canvas g.overlay rect.sel-box");
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text x="20" y="50" font-size="14">hello</text></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");
  await page.locator(".layerlist .row-btn").filter({ hasText: "text" }).click();

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

test("a source-defined gradient fill (url(#id)) shows its stops, not the raw url", async ({
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
  await page
    .locator("textarea")
    .fill(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><linearGradient id="a"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs><rect x="10" y="10" width="80" height="80" fill="url(#a)"/></svg>`,
    );
  await page.keyboard.press("Meta+Enter");
  await page.keyboard.press("v");
  await page.locator(".layerlist .row-btn").first().click();

  // The fill paint resolves the imported gradient: its "linear" mode is active + a read-only
  // preview bar shows (instead of a solid ColorInput holding the literal "url(#a)" string).
  const fill = page.locator(".paint").filter({ hasText: "fill" });
  await expect(fill.getByRole("button", { name: "linear", exact: true })).toHaveClass(/active/);
  await expect(fill.locator(".bar.readonly")).toBeVisible();

  // Adopting it (pick linear) seeds an editable model gradient from the imported stops → the
  // fill repoints to a nib `grad-…` id and the editable stop bar appears.
  await fill.getByRole("button", { name: "linear", exact: true }).click();
  await expect(page.locator('svg.canvas g.artwork path[fill^="url(#grad-"]')).toHaveCount(1);
  await expect(fill.locator(".marker")).toHaveCount(2);

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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "ungroup" }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await page.getByRole("button", { name: "paste svg", exact: true }).click();
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
  await expect(page.locator("svg.canvas g.overlay rect.sel-box")).toBeAttached();

  // Rotate via the knob above the box → the path geometry (its `d`) changes.
  const beforeD = await page.locator("svg.canvas g.artwork path").getAttribute("d");
  const knob = await page.locator("svg.canvas g.overlay .rotate-knob").boundingBox();
  if (!knob) throw new Error("no rotate knob");
  await page.mouse.move(knob.x + knob.width / 2, knob.y + knob.height / 2);
  await page.mouse.down();
  await page.mouse.move(knob.x + 45, knob.y + 30);
  await page.mouse.up();
  await expect(page.locator("svg.canvas g.artwork path")).not.toHaveAttribute("d", beforeD ?? "");

  // Styling round-trips through the core: set the stroke cap and see it on the element.
  await page
    .locator(".segrow")
    .filter({ hasText: "cap" })
    .getByRole("button", { name: "round" })
    .click();
  await expect(page.locator("svg.canvas g.artwork path")).toHaveAttribute("stroke-linecap", "round");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});
