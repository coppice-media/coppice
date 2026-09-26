# Contributing to Coppice

Thanks for helping improve Coppice. Use the repository issue tracker to discuss
bugs and proposed changes before investing in large or cross-cutting work.

## Development workspaces

JavaScript and Svelte changes belong to the active Bun workspaces: `home/`,
`editor/`, `docs/`, and `packages/stump-ui/`. The repository pins Bun 1.4.1;
there is no Node.js workspace or CI runtime.

From the repository root:

```sh
bun install --frozen-lockfile
bun run check-types
bun run home build
bun run editor build
bun run docs build
```

Regenerate GraphQL operations for the workspace you changed:

```sh
bun run home codegen
bun run editor codegen
bun run --filter @stump/ui codegen
```

The server and crates are Rust/Cargo workspaces. Follow the existing crate or
application conventions, include relevant tests and documentation, and run the
checks appropriate to your change.

## Pull requests

Keep changes focused and describe their behavior and verification clearly.
Open a pull request against the repository's default branch. Respond to review
feedback and keep the required CI checks passing before merge.
