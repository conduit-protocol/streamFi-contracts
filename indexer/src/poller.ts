import type { Pool, PoolClient } from "pg";
import { foldEvent, type RawEvent } from "./handlers.js";
import { retryWithBackoff, sleep, type RetryPolicy } from "./retry.js";

/**
 * Poller — fetches ledger events since the DB cursor and folds them into
 * derived tables.
 *
 * Crash-safety fix (this batch):
 * ------------------------------
 * Previously the poller did:
 *
 *   await ingestPage(pool, events);   // commits folds + raw_events
 *   await saveCursor(pool, nextLedger); // separate commit
 *
 * A crash between those two steps left the derived tables ahead of the
 * cursor. On restart the same page was re-fetched and re-folded, double-
 * counting unless every handler is idempotent. Idempotent upserts close half
 * the gap, but the correct fix is to make the cursor advance atomically with
 * the page it describes.
 *
 * Now both happen inside a single Postgres transaction:
 *
 *   BEGIN;
 *   ingestPage(client, events);  // raw_events inserts + foldEvent calls
 *   saveCursor(client, nextLedger);
 *   COMMIT;
 *
 * If the process crashes before COMMIT, the whole page rolls back and will
 * be re-fetched cleanly on the next run. If COMMIT succeeds, cursor and
 * folds are in sync. No separate timing fix is needed beyond this.
 *
 * Alternative considered: making saveCursor idempotent / compare-and-swap
 * without a transaction. Rejected — it still leaves a window where folds
 * are committed but the cursor isn't, and relies on every future handler
 * being idempotent. The transactional approach is strictly stronger and
 * costs one extra round-trip (the BEGIN/COMMIT) per page, which is
 * negligible next to the Horizon/RPC fetch.
 *
 * Retry / backoff (issue #569):
 * -----------------------------
 * Two different failure classes, two different strategies — both explicit:
 *
 *   fetch (RPC)   — retried *inside* pollOnce via `retryWithBackoff`
 *                   (exponential, default 3 attempts: 250ms, 500ms). A
 *                   transient RPC error never surfaces as a poll failure.
 *   ingest (DB)   — NOT retried inside pollOnce: the transaction has already
 *                   rolled back, so the clean retry is a fresh BEGIN. That
 *                   retry happens on the next loop iteration, where
 *                   startPollLoop catches the throw and sleeps
 *                   min(intervalMs, 2000) before polling again.
 *
 * The loop itself never dies on a failure: startPollLoop catches everything,
 * logs it, backs off, and polls again — so neither a bad RPC response nor a
 * Postgres blip can leave the worker "running but stalled" (see
 * test/poller.test.ts).
 */

export async function getCursor(client: PoolClient): Promise<number> {
  const res = await client.query("SELECT last_ledger FROM cursor WHERE id = 1");
  if (res.rows.length === 0) return 0;
  return Number(res.rows[0].last_ledger);
}

/**
 * Save the cursor inside the caller's transaction.
 * MUST be called with the same PoolClient that ran ingestPage, within the
 * same BEGIN/COMMIT block. Passing a Pool and doing an implicit separate
 * transaction reintroduces the split-commit bug — hence the PoolClient
 * signature.
 */
export async function saveCursor(
  client: PoolClient,
  ledger: number
): Promise<void> {
  await client.query(
    `INSERT INTO cursor (id, last_ledger, updated_at)
     VALUES (1, $1, NOW())
     ON CONFLICT (id) DO UPDATE
       SET last_ledger = GREATEST(cursor.last_ledger, EXCLUDED.last_ledger),
           updated_at = NOW()`,
    [ledger]
  );
}

export async function ingestPage(
  client: PoolClient,
  events: RawEvent[]
): Promise<void> {
  for (const ev of events) {
    // Persist raw event idempotently — UNIQUE (ledger, tx_hash, event_type, contract_id)
    await client.query(
      `INSERT INTO raw_events (ledger, tx_hash, event_type, contract_id, topics, data)
       VALUES ($1, $2, $3, $4, $5::jsonb, $6::jsonb)
       ON CONFLICT (ledger, tx_hash, event_type, contract_id) DO NOTHING`,
      [
        ev.ledger,
        ev.txHash,
        ev.eventType,
        ev.contractId,
        JSON.stringify(ev.topics ?? null),
        JSON.stringify(ev.data),
      ]
    );

    // Fold into derived tables (idempotent upserts — see handlers.ts).
    await foldEvent(client, ev);
  }
}

/**
 * Fetch events from the upstream ledger source.
 * In production this calls Horizon / Soroban RPC. Tests inject a mock.
 */
export type FetchEventsFn = (
  fromLedger: number,
  limit: number
) => Promise<RawEvent[]>;

export interface PollOnceOptions {
  /**
   * Retry policy for the RPC fetch step (issue #569). Defaults to
   * {@link DEFAULT_RETRY} — 3 attempts with 250ms/500ms exponential backoff.
   */
  retry?: RetryPolicy;
  /** Aborts an in-flight backoff sleep so shutdown is not delayed. */
  signal?: AbortSignal;
}

export async function pollOnce(
  pool: Pool,
  fetchEvents: FetchEventsFn,
  limit = 100,
  opts: PollOnceOptions = {}
): Promise<{ fetched: number; nextLedger: number | null }> {
  // Read cursor outside the ingest transaction (it's just the start point).
  const cursorClient = await pool.connect();
  let fromLedger: number;
  try {
    fromLedger = await getCursor(cursorClient);
  } finally {
    cursorClient.release();
  }

  // RPC fetch step — explicit retry-with-backoff so one transient upstream
  // failure (rate limit, 5xx, connection reset) does not fail the poll.
  // Exhausted retries rethrow; startPollLoop catches, backs off, and tries
  // the same cursor again — the cursor never advances on a failed fetch.
  const events = await retryWithBackoff(
    () => fetchEvents(fromLedger + 1, limit),
    { ...opts.retry, signal: opts.signal, label: "fetchEvents" }
  );
  if (events.length === 0) {
    return { fetched: 0, nextLedger: null };
  }

  const nextLedger = Math.max(...events.map((e) => e.ledger));

  // Atomic ingest + cursor advance — the core fix of this file.
  const client = await pool.connect();
  try {
    await client.query("BEGIN");
    await ingestPage(client, events);
    await saveCursor(client, nextLedger);
    await client.query("COMMIT");
  } catch (err) {
    await client.query("ROLLBACK");
    throw err;
  } finally {
    client.release();
  }

  return { fetched: events.length, nextLedger };
}

/**
 * Long-running poll loop used by the worker. Polls every `intervalMs`
 * until the process is terminated (SIGTERM/SIGINT) or the signal aborts. Each iteration is
 * independently transactional via pollOnce.
 *
 * Failure handling (issue #569): the loop never exits on a failure. A fetch
 * failure is first retried inside pollOnce (exponential backoff); if that is
 * exhausted — or the *ingest* half failed and rolled back — the error lands
 * here, is logged, and the loop sleeps min(intervalMs, 2000) before polling
 * the same cursor again. So the only ways this loop stops are abort or
 * process death; it cannot silently stall while looking healthy.
 */
export async function startPollLoop(
  pool: Pool,
  fetchEvents: FetchEventsFn,
  opts: {
    intervalMs?: number;
    limit?: number;
    signal?: AbortSignal;
    /** Retry policy forwarded to the fetch step of pollOnce. */
    retry?: RetryPolicy;
  } = {}
): Promise<void> {
  const intervalMs = opts.intervalMs ?? 5_000;
  const limit = opts.limit ?? 100;
  const signal = opts.signal;

  while (!signal?.aborted) {
    try {
      const { fetched } = await pollOnce(pool, fetchEvents, limit, {
        retry: opts.retry,
        signal,
      });
      if (signal?.aborted) break;
      if (fetched === 0) {
        await sleep(intervalMs, signal);
      }
    } catch (err) {
      if (signal?.aborted) break;
      console.error("[poller] pollOnce failed:", err);
      // Back off briefly before retrying — avoids tight crash-loop if the
      // DB or upstream is down.
      await sleep(Math.min(intervalMs, 2000), signal);
    }
  }
}

