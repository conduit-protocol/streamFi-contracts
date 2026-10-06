import type { PoolClient } from 'pg'
import type { ChainEvent } from './types.js'

function num(value: unknown): number {
  return typeof value === 'number' ? value : Number(value)
}

function str(value: unknown): string {
  return value === undefined || value === null ? '' : String(value)
}

/** i128 amount field → decimal string, or null when the event carries none. */
function amountOrNull(value: unknown): string | null {
  if (value === undefined || value === null || value === '') return null
  return String(value)
}

/**
 * Stream identity for a stream event: the decoder-supplied id when it has one
 * (factory `stream_id`), otherwise the emitting DripStream contract address —
 * which is unique per stream, so `streams.id` / `stream_events.stream_id` are
 * always populated.
 */
function streamIdOf(ev: ChainEvent): string {
  const f = ev.fields
  return str(f.stream_id ?? f.streamId ?? f.id ?? ev.contractId)
}

/**
 * Folds a single decoded event into the derived tables.
 *
 * Two families of folds:
 *  - DripStream / DripFactory events (`streams`, `stream_events`) — issues
 *    #566/#567. Idempotent upserts: re-delivering a page rewrites the same
 *    row, so a crash between ingest and checkpoint cannot double-count.
 *  - DAO-voting events (`loan_proposals`, `treasury_proposals`) — legacy
 *    scaffolding. These folds are additive (increments), not idempotent.
 *    Re-delivering an already-applied event double-counts it. That's a known
 *    gap tracked separately — see "Non-idempotent derived-table folds" in the
 *    top-level README — and is intentionally not fixed as part of this scaffold.
 *
 * Type tags: the switch accepts both the README event names
 * (`stream_withdrawn`, `stream_cancelled`, …) and the raw `symbol_short!`
 * topic names emitted on-chain (`withdrawn`, `cancelled`, `created`, …) as
 * documented on {@link ChainEvent.type} / `types.ts:EventType`, and always
 * stores the canonical README name in `stream_events.event_type`.
 */
export async function applyEvent(client: PoolClient, ev: ChainEvent): Promise<void> {
  switch (ev.type) {
    // Stream creation (issue #567) — `DripStream::created`.
    case 'created':
    case 'stream_created':
      return handleStreamCreated(client, ev)
    // Stream lifecycle (issue #566) — README's DripStream events table.
    case 'withdrawn':
    case 'stream_withdrawn':
      return handleStreamWithdrawn(client, ev)
    case 'cancelled':
    case 'stream_cancelled':
      return handleStreamCancelled(client, ev)
    case 'force_cxl':
    case 'force_cancelled':
      return handleStreamForceCancelled(client, ev)
    case 'paused':
    case 'stream_paused':
      return handleStreamPaused(client, ev)
    case 'resumed':
    case 'stream_resumed':
      return handleStreamResumed(client, ev)
    case 'topped_up':
    case 'stream_topped_up':
      return handleStreamToppedUp(client, ev)
    case 'clawback':
    case 'stream_clawback':
      return handleStreamClawback(client, ev)
    case 'xfer_rec':
      return handleXferRec(client, ev)
    case 'loan_vote':
      return loan_vote(client, ev)
    case 'treasury_vote':
      return treasury_vote(client, ev)
    case 'treasury_reveal':
      return treasury_reveal(client, ev)
    default:
      // Log (don't throw) on anything we don't recognise: today that's every
      // event type except the folds above. Once real DripStream/DripFactory
      // handlers land here, a future contract upgrade that adds a new event
      // type must show up in the logs instead of disappearing into this
      // indexer silently (issue #578).
      console.error(
        JSON.stringify({
          level: 'warn',
          msg: 'applyEvent: unrecognized event type, dropping',
          type: ev.type,
          ledger: ev.ledger,
          contractId: ev.contractId,
          txHash: ev.txHash,
        }),
      )
      return
  }
}

// ---------------------------------------------------------------------------
// Stream handlers (issues #566, #567)
// ---------------------------------------------------------------------------

/**
 * Stream creation → `streams` (issue #567).
 *
 * `created_at` is left to the column default: on a re-delivered event the
 * ON CONFLICT clause rewrites the parameters but never the creation time, so
 * "streams created in the last 24h" stays stable across replays.
 */
export async function handleStreamCreated(client: PoolClient, ev: ChainEvent): Promise<void> {
  const f = ev.fields
  await client.query(
    `INSERT INTO streams (id, sender, recipient, token, rate_per_second, start_time, end_time)
     VALUES ($1, $2, $3, $4, $5, $6, $7)
     ON CONFLICT (id) DO UPDATE
       SET sender          = EXCLUDED.sender,
           recipient       = EXCLUDED.recipient,
           token           = EXCLUDED.token,
           rate_per_second = EXCLUDED.rate_per_second,
           start_time      = EXCLUDED.start_time,
           end_time        = EXCLUDED.end_time`,
    [
      streamIdOf(ev),
      str(f.sender),
      str(f.recipient),
      str(f.token),
      str(f.rate_per_second ?? 0),
      num(f.start_time ?? 0),
      num(f.end_time ?? 0),
    ],
  )
}

/**
 * Append one row to `stream_events` (issue #566).
 *
 * The primary key `(ledger, tx_hash, event_type, stream_id)` is exactly the
 * page-re-delivery key: folding the same event twice rewrites the same row
 * (same `amount`, same `payload`) instead of appending a duplicate.
 */
async function recordStreamEvent(
  client: PoolClient,
  ev: ChainEvent,
  eventType: string,
  amount: string | null = null,
): Promise<void> {
  await client.query(
    `INSERT INTO stream_events (stream_id, event_type, ledger, tx_hash, amount, payload)
     VALUES ($1, $2, $3, $4, $5, $6::jsonb)
     ON CONFLICT (ledger, tx_hash, event_type, stream_id) DO UPDATE
       SET amount  = EXCLUDED.amount,
           payload = EXCLUDED.payload`,
    [
      streamIdOf(ev),
      eventType,
      num(ev.ledger ?? 0),
      str(ev.txHash),
      amount,
      JSON.stringify(ev.fields ?? {}),
    ],
  )
}

export async function handleStreamWithdrawn(client: PoolClient, ev: ChainEvent): Promise<void> {
  return recordStreamEvent(client, ev, 'stream_withdrawn', amountOrNull(ev.fields.amount))
}

export async function handleStreamCancelled(client: PoolClient, ev: ChainEvent): Promise<void> {
  return recordStreamEvent(client, ev, 'stream_cancelled', amountOrNull(ev.fields.refund_amount))
}

export async function handleStreamForceCancelled(client: PoolClient, ev: ChainEvent): Promise<void> {
  // `force_cxl` is recorded under its own event_type (per types.ts) so a
  // consumer can tell sender-cancel from recipient force-cancel without
  // correlating the transaction signer.
  return recordStreamEvent(client, ev, 'stream_force_cancelled', amountOrNull(ev.fields.refund_amount))
}

export async function handleStreamPaused(client: PoolClient, ev: ChainEvent): Promise<void> {
  return recordStreamEvent(client, ev, 'stream_paused')
}

export async function handleStreamResumed(client: PoolClient, ev: ChainEvent): Promise<void> {
  return recordStreamEvent(client, ev, 'stream_resumed')
}

export async function handleStreamToppedUp(client: PoolClient, ev: ChainEvent): Promise<void> {
  return recordStreamEvent(client, ev, 'stream_topped_up', amountOrNull(ev.fields.amount))
}

export async function handleStreamClawback(client: PoolClient, ev: ChainEvent): Promise<void> {
  return recordStreamEvent(client, ev, 'stream_clawback', amountOrNull(ev.fields.amount))
}

export async function handleXferRec(client: PoolClient, ev: ChainEvent): Promise<void> {
  return recordStreamEvent(client, ev, 'xfer_rec')
}

async function loan_vote(client: PoolClient, ev: ChainEvent): Promise<void> {
  const f = ev.fields
  const column = f.support === true ? 'votes_for' : 'votes_against'
  await client.query(
    `INSERT INTO loan_proposals (id, ${column})
     VALUES ($1, 1)
     ON CONFLICT (id) DO UPDATE
       SET ${column} = loan_proposals.${column} + 1,
           updated_at = now()`,
    [num(f.proposal_id)],
  )
}

async function treasury_vote(client: PoolClient, ev: ChainEvent): Promise<void> {
  const f = ev.fields
  const column = f.support === true ? 'votes_for' : 'votes_against'
  await client.query(
    `INSERT INTO treasury_proposals (id, ${column})
     VALUES ($1, 1)
     ON CONFLICT (id) DO UPDATE
       SET ${column} = treasury_proposals.${column} + 1,
           updated_at = now()`,
    [num(f.proposal_id)],
  )
}

async function treasury_reveal(client: PoolClient, ev: ChainEvent): Promise<void> {
  const f = ev.fields
  await client.query(
    `INSERT INTO treasury_proposals (id, revealed)
     VALUES ($1, 1)
     ON CONFLICT (id) DO UPDATE
       SET revealed = treasury_proposals.revealed + 1,
           updated_at = now()`,
    [num(f.proposal_id)],
  )
}
