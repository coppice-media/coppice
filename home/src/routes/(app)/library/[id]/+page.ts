// A library id is only known at runtime, so this page is served by the SPA
// fallback (`adapter-static`'s `fallback: index.html`) instead of being
// prerendered like the static screens.
export const prerender = false;
