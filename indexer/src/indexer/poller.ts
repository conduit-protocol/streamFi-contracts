import type { Pool } from 'pg'
import { applyEvent } from './handlers.js'
import type { ChainEvent, EventSource } from './types.js'

export interface PollerOptions {
  pool: Pool
  source: EventSource
  pageSize: number
  pollIntervalMs: number
}

export interface Cursor {
  token: string | null
  lastLedger: number
}

export async function readCursor(pool: Pool): Promise<Cursor> {
  const { rows } = await pool.query<{ token: string | null; last_ledger: string }>(
    'SELECT token, last_ledger FROM indexer_cursor WHERE id = 1',
  )
  if (rows.length === 0) {
    return { token: null, lastLedger: 0 }
  }
  return { token: rows[0].token, lastLedger: Number(rows[0].last_ledger) }
}

export async function saveCursor(pool: Pool, cursor: Cursor): Promise<void> {
  await pool.query(
    `UPDATE indexer_cursor SET token = $1, last_ledger = $2, updated_at = now() WHERE id = 1`,
    [cursor.token, cursor.lastLedger],
  )
}

/**
 * Ingests one page of events: raw insert (idempotent) + derived-table fold, in a single
 * transaction. The caller is responsible for persisting the cursor afterwards.
 */
export async function ingestPage(pool: Pool, events: ChainEvent[]): Promise<void> {
  const client = await pool.connect()
  try {
    await client.query('BEGIN')
    for (const ev of events) {
      await client.query(
        `INSERT INTO raw_events (id, ledger, event_type, payload)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (id) DO NOTHING`,
        [ev.id, ev.ledger, ev.type, JSON.stringify(ev.fields)],
      )
      await applyEvent(client, ev)
    }
    await client.query('COMMIT')
  } catch (err) {
    await client.query('ROLLBACK')
    throw err
  } finally {
    client.release()
  }
}

export class Poller {
  private readonly pool: Pool
  private readonly source: EventSource
  private readonly pageSize: number
  private readonly pollIntervalMs: number
  private timer: NodeJS.Timeout | null = null
  private stopped = false

  constructor(opts: PollerOptions) {
    this.pool = opts.pool
    this.source = opts.source
    this.pageSize = opts.pageSize
    this.pollIntervalMs = opts.pollIntervalMs
  }

  async start(): Promise<void> {
    this.stopped = false
    await this.tick()
  }

  stop(): void {
    this.stopped = true
    if (this.timer) {
      clearTimeout(this.timer)
      this.timer = null
    }
  }

  private async tick(): Promise<void> {
    if (this.stopped) return

    const cursor = await readCursor(this.pool)
    const page = await this.source.fetchPage(cursor.token, this.pageSize)

    if (page.events.length > 0) {
      await ingestPage(this.pool, page.events)
    }
    await saveCursor(this.pool, { token: page.nextToken, lastLedger: page.lastLedger })

    if (!this.stopped) {
      this.timer = setTimeout(() => {
        void this.tick()
      }, this.pollIntervalMs)
    }
  }
}
