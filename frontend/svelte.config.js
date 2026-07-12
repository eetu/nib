import adapter from "@sveltejs/adapter-static";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  compilerOptions: {
    // Force runes mode (Svelte 5). Can be removed in Svelte 6.
    runes: ({ filename }) => (filename.split(/[/\\]/).includes("node_modules") ? undefined : true),
  },
  kit: {
    // Pure SPA: no server-side logic. Output to dist/ to match the family
    // convention so a future Rust backend embeds it and serves index.html as
    // the fallback for every unmatched path. See spa-frontend / rust-axum.
    adapter: adapter({
      pages: "dist",
      assets: "dist",
      fallback: "index.html",
      precompress: false,
      strict: true,
    }),
  },
};

export default config;
