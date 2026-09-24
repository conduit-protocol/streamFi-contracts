-- StreamFi indexer — initial schema (legacy one-shot).
-- DEPRECATED: use versioned migrations in `db/migrations/` via `npm run migrate`
-- (`indexer/src/db/migrate.ts` or `node db/migrate.js up`).
-- This file is retained for reference and for fresh local `psql` bootstraps
-- without the runner. New schema changes must go in `db/migrations/*.sql`
-- with a monotonically increasing numeric prefix.
--
-- To apply with the runner:  DATABASE_URL=postgres://... npm run --prefix indexer migrate
-- Legacy one-shot (no history):  psql "$DATABASE_URL" -f db/schema.sql

-- Streams — one row per DripStream contract instance
CREATE TABLE IF NOT EXISTS streams (
    stream_id        BIGINT PRIMARY KEY,
    contract_address TEXT NOT NULL UNIQUE,
    sender           TEXT NOT NULL,
    recipient        TEXT NOT NULL,
    token            TEXT NOT NULL,
    rate_per_second  TEXT NOT NULL, -- i128 decimal string
    start_time       BIGINT NOT NULL,
    end_time         BIGINT NOT NULL, -- 0 = open-ended
    status           TEXT NOT NULL DEFAULT 'active', -- active | paused | cancelled
    withdrawn_total  TEXT NOT NULL DEFAULT '0',
    created_at_ledger BIGINT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_streams_sender    ON streams(sender);
CREATE INDEX IF NOT EXISTS idx_streams_recipient ON streams(recipient);

-- Raw events — append-only log of every Soroban event folded
CREATE TABLE IF NOT EXISTS raw_events (
    id          BIGSERIAL PRIMARY KEY,
    ledger      INTEGER NOT NULL,
    tx_hash     TEXT NOT NULL,
    contract_id TEXT NOT NULL,
    type        TEXT NOT NULL, -- e.g. created, withdrawn, cancelled, xfer_rec
    sequence    BIGINT,        -- per-stream monotonic seq, NULL for factory events
    fields      JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tx_hash, type, sequence)
);
-- The type index speeds up "all withdrawals for stream X" and the proposed
-- `raw_events_type_idx` change elsewhere in this batch.
CREATE INDEX IF NOT EXISTS idx_raw_events_type         ON raw_events(type);
CREATE INDEX IF NOT EXISTS idx_raw_events_contract_ledger ON raw_events(contract_id, ledger);
CREATE INDEX IF NOT EXISTS idx_raw_events_ledger        ON raw_events(ledger);

-- Indexer cursor — high-water mark for the poller (single row, id=1)
CREATE TABLE IF NOT EXISTS indexer_cursor (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    last_ledger INTEGER NOT NULL,
    next_token  TEXT,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Migration history — managed by the migration runner, not hand-edited
CREATE TABLE IF NOT EXISTS schema_migrations (
    version    TEXT PRIMARY KEY, -- e.g. 001_initial
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
