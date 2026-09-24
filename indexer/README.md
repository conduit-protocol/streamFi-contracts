# StreamFi Indexer

Tails Soroban `getEvents` and projects stream state into Postgres.

## Run

```bash
# Migrations (replaces legacy `psql "$DATABASE_URL" -f db/schema.sql`)
DATABASE_URL=postgres://user:pass@localhost:5432/streamfi npm run migrate
# status / down
DATABASE_URL=... npx tsx src/db/migrate.ts status
DATABASE_URL=... npx tsx src/db/migrate.ts down

# Worker (poll + HTTP)
PORT=3000 START_LEDGER=1 POLL_INTERVAL_MS=5000 npm run start
# or dev:
npm run dev
```

## Endpoints

- `GET /healthz` — `{ status: "ok"|"degraded", lastSuccessfulPollTimestamp, lastSuccessfulPollIso, currentCursor: { lastLedger, nextToken }, uptimeSeconds, version }`
  Returns 200 when healthy, 503 when `lastSuccessfulPoll` is older than `HEALTHZ_STALE_MS` (default 5m). K8s `livenessProbe`/`readinessProbe` should hit this. `GET /readyz` is an alias.
  Alternative signal: set `HEALTHZ_FILE=/tmp/indexer.ready` — worker touches the file on each successful poll; orchestrator can check `mtime`.

- `GET /metrics` — Prometheus text format:
  ```
  indexer_pages_processed_total
  indexer_events_folded_total
  indexer_fold_failures_total
  indexer_last_successful_poll_timestamp_seconds
  indexer_current_cursor_ledger
  ```
  Counter triples (`pages_processed`, `events_folded`, `fold_failures`) are also emitted as structured JSON log lines on stderr (`msg=indexer_tick`) for dashboards without scraping.

## Contract

See `src/indexer/types.ts` for the `SorobanEventSource` pagination contract (`nextToken` opaque, `lastLedger` inclusive) and the expected `fields` shape per `ev.type`. That file is the entire interface between the poller and the event source implementation that will replace `StubSorobanEventSource`.
