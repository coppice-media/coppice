// The reader is opened with a book id that only exists at runtime, so there is
// nothing for the static adapter to crawl. `fallback: 'index.html'` (and the
// server's own SPA fallback under `/app`) serves this route on demand.
export const prerender = false;
