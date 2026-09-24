-- StreamFi indexer schema.
--
-- Two layers:
--   1. Raw event log (`raw_events`) — append-only, idempotent via ON CONFLICT DO NOTHING.
--   2. Derived tables (`loan_proposals`, `treasury_proposals`) — folded from raw events by
--      src/indexer/handlers.ts. Folding is currently additive (increments), which is NOT
--      idempotent — see the "Known gaps" section of the top-level README before relying on
--      these tallies for anything safety-critical.

CREATE TABLE IF NOT EXISTS indexer_cursor (
  id          integer PRIMARY KEY,
  token       text,
  last_ledger bigint NOT NULL DEFAULT 0,
  updated_at  timestamptz NOT NULL DEFAULT now()
);

-- Single row: the poller's resume position. Seeded so a fresh worker has a row to read.
INSERT INTO indexer_cursor (id, token, last_ledger)
VALUES (1, NULL, 0)
ON CONFLICT (id) DO NOTHING;

CREATE TABLE IF NOT EXISTS raw_events (
  id         text PRIMARY KEY,
  ledger     bigint NOT NULL,
  event_type text NOT NULL,
  payload    jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS raw_events_ledger_idx ON raw_events (ledger);

CREATE TABLE IF NOT EXISTS loan_proposals (
  id            bigint PRIMARY KEY,
  votes_for     bigint NOT NULL DEFAULT 0,
  votes_against bigint NOT NULL DEFAULT 0,
  updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS treasury_proposals (
  id            bigint PRIMARY KEY,
  votes_for     bigint NOT NULL DEFAULT 0,
  votes_against bigint NOT NULL DEFAULT 0,
  revealed      integer NOT NULL DEFAULT 0,
  updated_at    timestamptz NOT NULL DEFAULT now()
);
