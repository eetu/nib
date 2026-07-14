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
