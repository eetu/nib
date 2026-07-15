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
  await expect(artwork).toHaveAttribute("d", /M40 120/);

  // Draw a two-node path with the pen (beginPath + appendNode ops → drawn render).
  await page.keyboard.press("p");
  const box = await page.locator("svg.canvas").boundingBox();
  if (!box) throw new Error("canvas has no bounding box");
  await page.mouse.click(box.x + box.width * 0.3, box.y + box.height * 0.35);
  await page.mouse.click(box.x + box.width * 0.6, box.y + box.height * 0.6);
  await page.keyboard.press("Escape");
  await expect(page.locator("svg.canvas g.drawn path")).toHaveCount(1);

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

  // One drawn path (the rectangle), and it round-trips to a 4-corner closed `d`.
  const rect = page.locator("svg.canvas g.drawn path");
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
  await expect(page.locator("svg.canvas g.drawn path")).toHaveCount(2);

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
  await expect(page.locator("svg.canvas g.drawn path")).toHaveCount(0);

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
  await expect(page.locator("svg.canvas g.drawn path")).toHaveAttribute("fill", /url\(#grad-/);

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

  // The stroked line is replaced by a drawn fill shape whose fill is the old stroke colour.
  const drawn = page.locator("svg.canvas g.drawn path");
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
  // The offset result is a new drawn path (source kept).
  await expect(page.locator("svg.canvas g.drawn path")).toHaveCount(1);

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
  const d = await page.locator("svg.canvas g.drawn path").getAttribute("d");
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
  await expect(page.locator("svg.canvas g.drawn path")).toHaveCount(2);

  // Select both, then union → one result path replaces them.
  await page.keyboard.press("v");
  const rows = page.locator(".layerlist .row-btn");
  await rows.nth(0).click();
  await rows.nth(1).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "union", exact: true }).click();
  await expect(page.locator("svg.canvas g.drawn path")).toHaveCount(1);

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

  // The computed result renders, and BOTH operands survive (non-destructive — vs the
  // destructive boolean above which collapses to one path).
  const result = page.locator("svg.canvas g.booleans path");
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

  // Select + nudge → the whole rect moves. It repaints declaratively as a <path> on the canvas
  // (its source <rect> hides), but a form-preserving move keeps it a <rect> on export (re-fit).
  await rows.nth(0).click();
  for (let i = 0; i < 3; i++) await page.keyboard.press("ArrowRight");
  await expect(page.locator("svg.canvas g.drawn path")).toHaveCount(1);
  await expect(page.locator("svg.canvas g.artwork rect")).toHaveAttribute("display", "none");

  // Source (= export) still has a <rect> (moved), not a <path> — clean markup preserved.
  await page.getByRole("button", { name: "source" }).click();
  const src = await page.locator(".sourceview textarea").inputValue();
  expect(src).toContain("<rect");
  expect(src).toContain('x="23"'); // nudged +3
  expect(src).not.toContain("<path");

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
