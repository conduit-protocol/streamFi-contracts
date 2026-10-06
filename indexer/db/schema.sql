-- StreamFi indexer schema.
--
-- Two layers:
--   1. Raw event log (`raw_events`) — append-only, idempotent via ON CONFLICT DO NOTHING.
--   2. Derived tables (`streams`, `stream_events`, `loan_proposals`, `treasury_proposals`) —
--      folded from raw events by src/indexer/handlers.ts. The stream folds are idempotent
--      upserts; the DAO-voting folds are additive (increments) and are NOT idempotent —
--      see the "Known gaps" section of the top-level README before relying on those tallies
--      for anything safety-critical.
--
-- `streams` / `stream_events` cover the protocol the repo actually ships (DripStream /
-- DripFactory — issues #566, #567); the DAO-voting tables are legacy scaffolding.

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

-- Streams — one row per DripStream, folded from the stream-creation event
-- (`DripStream::created`, emitted by DripFactory::create_stream deployments;
-- issue #567). Answers "which streams exist / who created them / what were
-- the initial parameters" without scanning and decoding raw_events.
--
-- `id` is the stream's identifier as delivered by the event decoder: the
-- factory's monotonically increasing stream id when the decoder supplies one,
-- otherwise the DripStream contract address (which is unique per stream).
CREATE TABLE IF NOT EXISTS streams (
  id              text PRIMARY KEY,
  sender          text NOT NULL,
  recipient       text NOT NULL,
  token           text NOT NULL,
  rate_per_second text NOT NULL, -- i128 decimal string — exceeds bigint on-chain
  start_time      bigint NOT NULL,
  end_time        bigint NOT NULL, -- 0 = open-ended
  created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS streams_created_at_idx ON streams (created_at);
CREATE INDEX IF NOT EXISTS streams_sender_idx ON streams (sender);
CREATE INDEX IF NOT EXISTS streams_recipient_idx ON streams (recipient);

-- Stream lifecycle events — one row per (ledger, tx, type, stream) for the
-- DripStream events the README documents (withdrawn, cancelled, paused,
-- resumed, topped_up, clawback, xfer_rec; issue #566). Unlike the additive
-- DAO tallies this is an event log, so its upsert is "same event seen again →
-- rewrite the same row" (idempotent under page re-delivery).
--
-- `amount` is the event's i128 decimal string (withdrawn amount / refund /
-- top-up / clawback) when the event carries one; `payload` keeps every field
-- for events without one and as the audit copy for events with one.
CREATE TABLE IF NOT EXISTS stream_events (
  stream_id  text NOT NULL,
  event_type text NOT NULL,
  ledger     bigint NOT NULL,
  tx_hash    text NOT NULL,
  amount     text,
  payload    jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (ledger, tx_hash, event_type, stream_id)
);

CREATE INDEX IF NOT EXISTS stream_events_stream_id_idx ON stream_events (stream_id);
CREATE INDEX IF NOT EXISTS stream_events_created_at_idx ON stream_events (created_at);
CREATE INDEX IF NOT EXISTS stream_events_type_idx ON stream_events (event_type);

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
