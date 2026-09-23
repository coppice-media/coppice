# web

This is the React SPA served by the [Coppice server](../server) by default. It is
the primary management and general user interface for Coppice.

The app is a slim wrapper around the shared [browser package](../../packages/browser),
which owns the web interface used by both browser-based shells.

## Testing

See [tests](./tests) for the playwright suite for the web app. A quick start includes:

```bash
# this assumes you have a Coppice server running on port 10801
bun install --frozen-lockfile
# optional install step if you need to install Playwright browsers
bun run --filter @stump/web e2e:install
bun run --filter @stump/web e2e
```
