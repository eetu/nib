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

  // Styling round-trips through the core: set the stroke cap and see it on the element.
  await page
    .locator(".segrow")
    .filter({ hasText: "cap" })
    .getByRole("button", { name: "round" })
    .click();
  await expect(page.locator("svg.canvas g.artwork path")).toHaveAttribute("stroke-linecap", "round");

  expect(errors, `console/page errors:\n${errors.join("\n")}`).toEqual([]);
});
