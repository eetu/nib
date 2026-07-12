// Pure SPA: no SSR, no prerender. The whole app is client-side (it touches the
// File System Access API and the DOM directly), and a future backend just
// serves the built files. See spa-frontend.
export const ssr = false;
export const prerender = false;
