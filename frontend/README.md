# frontend

This directory is a **components-only** library, not a standalone app.

## Relationship to `streamFi-app`

`streamFi-app` is the actual production frontend for Conduit/StreamFi — it
owns the build, routing, and deployment. This directory does not compete
with it; it exists so that Stellar/Soroban-facing UI pieces (stream
creation, wallet connection, fee estimation, validation) can be built and
unit-tested against the contracts in this repo, close to the Rust source
they call into, before being consumed elsewhere. Components here are meant
to be imported into `streamFi-app` (or ported into it), not run standalone.

## Layout

- `src/` — the UI components (`*.tsx`) and their co-located `*.module.css`
  files, plus `streams.ts` (the low-level Soroban SDK wrapper).
- `lib/` — framework-agnostic helpers (validation, fee estimation, the
  higher-level stream SDK wrapper) with their unit tests alongside them.

## Build tooling

There is intentionally no `vite.config.ts`, `next.config.js`, or other
bundler config here. `css-modules.d.ts` only teaches TypeScript how to type
`*.module.css` imports (`import styles from './Foo.module.css'`) — it does
not imply this directory produces its own build output.

These components (and their co-located `*.module.css` files) are expected
to be imported directly into a host application's existing build (e.g. a
Next.js or Vite app elsewhere in the Conduit stack), which supplies the
actual CSS Modules loader/bundler. If that assumption changes and this
directory needs to ship its own build, add the bundler config then —
until a host app exists, adding one here would be unused surface area to
maintain.

## Scripts

```bash
npm run typecheck   # tsc --noEmit
npm run lint        # eslint .
npm test            # vitest run
```

These are also run in CI on every push/PR that touches `frontend/` — see
`.github/workflows/ci.yml`.
