/** Build-time feature flag (Vite statically replaces `import.meta.env.VITE_*`, so it tree-shakes,
 *  and an unset var is simply `undefined` → off — no committed .env needed). When off, no backend
 *  code/UI ships: the SPA is the local-first file editor (the GitHub-Pages build). `just dev` and
 *  any backend-embedded build set `VITE_NIB_BACKEND=1`. */
export const BACKEND = import.meta.env.VITE_NIB_BACKEND === "1";
