# StreamFi Indexer

Polls contract events into Postgres: an append-only raw log (`raw_events`) plus derived
tables (`loan_proposals`, `treasury_proposals`) folded from that log.

This is scaffolding — the piece that's still missing is a real `EventSource`
(`src/indexer/chainEventSource.ts`) wired to a live Soroban RPC endpoint. Everything
around it (schema, cursor persistence, transaction boundaries, worker lifecycle) is real.

## Setup

```bash
npm install
cp .env.example .env   # set DATABASE_URL
psql "$DATABASE_URL" -f db/schema.sql
npm run dev             # or: npm run build && npm run start:worker
```

## Layout

- `src/worker.ts` — process entry point; owns the pg pool and shuts it down on
  SIGINT/SIGTERM.
- `src/indexer/poller.ts` — read cursor → fetch page → ingest → save cursor loop.
- `src/indexer/handlers.ts` — folds one decoded event into the derived tables.
- `src/indexer/chainEventSource.ts` — the chain integration point (currently a stub).
- `db/schema.sql` — `indexer_cursor`, `raw_events`, `loan_proposals`, `treasury_proposals`.

## Known gaps

- **Single indexer instance** — no leader election or multi-instance coordination. Running
  two `npm run start:worker` processes against the same database (e.g. an overlapping
  rolling deploy) means both read the same cursor and both fold the same page.
- **Non-idempotent derived-table folds** — `applyEvent` (`src/indexer/handlers.ts`) updates
  `votes_for` / `votes_against` with increments, not idempotent upserts, and
  `saveCursor` runs after `ingestPage` commits rather than in the same transaction
  (`src/indexer/poller.ts`). A crash between those two steps, or a second worker
  re-fetching an already-folded page, double-counts every vote in that page. The raw log
  is unaffected (`ON CONFLICT (id) DO NOTHING`), but nothing records that an event was
  already folded, so there's no way to detect or repair the drift after the fact short of
  rebuilding the derived tables from `raw_events`.

Both are tracked as follow-up work, not fixed as part of this scaffold.
