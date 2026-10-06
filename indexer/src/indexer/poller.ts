/**
 * Poller — fetches pages from a {@link SorobanEventSource}, folds each
 * event, and checkpoints the {@link Cursor}.
 *
 * ~105 lines before counters; now exposes three counters:
 *  - `pages_processed`   — pages fetched (including empty pages)
 *  - `events_folded`     — events successfully folded
 *  - `fold_failures`    — events that failed to fold (per-event)
 *
 * Counters are incremented in-memory via `metrics` and emitted as:
 *  - structured JSON log lines (stderr, once per tick)
 *  - Prometheus gauges/counters at `GET /metrics` (via `metrics.ts`)
 *
 * Health is reported via `health.ts` — `lastSuccessfulPollTimestamp` and
 * `currentCursor` power `GET /healthz`.
 *
 * Transient failures (issue #569): the `getEvents` fetch step runs through
 * `retryWithBackoff` (`../retry.ts`, exponential, default 3 attempts). If all
 * attempts fail the cursor is left untouched and the next tick retries it —
 * the loop never silently stops advancing because of one bad RPC response.
 */

import { Cursor, SorobanEventSource, GetEventsParams } from "./types.js";
import { fold } from "./fold.js";
import { retryWithBackoff, DEFAULT_RETRY, type RetryPolicy } from "../retry.js";
import { metrics } from "../metrics.js";
import { health } from "../health.js";

export interface PollerOptions {
  /** Event source (stub or real RPC-backed). */
  source: SorobanEventSource;
  /** Starting ledger when no checkpoint exists (factory deploy ledger). */
  startLedger: number;
  /** Poll interval in ms. Default 5000. */
  intervalMs?: number;
  /** Max events per page. Default 100 (matches on-chain MAX_PAGE_SIZE). */
  limit?: number;
  /**
   * Retry-with-backoff policy for the `getEvents` fetch step (issue #569).
   * Default: 3 attempts, 250ms base doubling to a 5s cap. Exhausted retries
   * are logged as an error and the *next tick* retries the same cursor —
   * a transient RPC/DB blip never stalls the loop silently.
   */
  retry?: RetryPolicy;
  /** Optional loader/saver for the high-water cursor (e.g. Postgres or file). */
  loadCursor?: () => Promise<Cursor | null>;
  saveCursor?: (cursor: Cursor) => Promise<void>;
}

export class Poller {
  private timer: NodeJS.Timeout | null = null;
  private running = false;
  private cursor: Cursor | null = null;

  constructor(private readonly opts: PollerOptions) {}

  /** Start periodic polling. Idempotent. */
  async start(): Promise<void> {
    if (this.running) return;
    this.running = true;

    if (this.opts.loadCursor) {
      try {
        this.cursor = await this.opts.loadCursor();
        if (this.cursor) {
          metrics.setCurrentCursorLedger(this.cursor.lastLedger);
          health.setCursor(this.cursor);
        }
      } catch (e) {
        console.error(JSON.stringify({ level: "warn", msg: "failed to load cursor", error: String(e) }));
      }
    }
    if (!this.cursor) {
      this.cursor = { lastLedger: this.opts.startLedger - 1, nextToken: null };
    }

    // Immediate tick, then interval
    await this.tick();
    this.timer = setInterval(() => {
      this.tick().catch((e) => {
        console.error(JSON.stringify({ level: "error", msg: "tick threw", error: String(e) }));
      });
    }, this.opts.intervalMs ?? 5000);
    if (this.timer.unref) this.timer.unref();
  }

  stop(): void {
    this.running = false;
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  private async tick(): Promise<void> {
    if (!this.cursor) return;

    const startLedger = this.cursor.nextToken !== null ? this.cursor.lastLedger : this.cursor.lastLedger + 1;
    // For continuation pages, startLedger is the same as previous; we track it via cursor.lastLedger
    // but the source expects the original range. Simplify: when nextToken != null we reuse the same
    // startLedger as the page that produced the token (stored in cursor.lastLedger for empty continuation?).
    // For scaffold we use cursor.lastLedger as start for next fetch; real implementation keeps range.
    const params: GetEventsParams = {
      startLedger,
      cursor: this.cursor.nextToken,
      limit: this.opts.limit ?? 100,
    };

    let page;
    const attempts = Math.max(1, this.opts.retry?.attempts ?? DEFAULT_RETRY.attempts);
    try {
      // Explicit retry-with-backoff around the RPC fetch step (issue #569).
      // Retry lines are logged by retryWithBackoff; a single transient
      // failure therefore costs `delay`, not the rest of the stream.
      page = await retryWithBackoff(() => this.opts.source.getEvents(params), {
        ...this.opts.retry,
        label: "getEvents",
      });
    } catch (e) {
      // Every attempt in this tick failed. Do not advance the cursor: the
      // next tick rebuilds the same params from the unchanged cursor and
      // tries again, so a transient failure only delays progress — the loop
      // keeps running (interval ticks are scheduled independently of a
      // thrown tick, see start()) and cannot stall silently.
      console.error(JSON.stringify({ level: "error", msg: "getEvents failed after retries", error: String(e), attempts, startLedger, cursor: this.cursor.nextToken }));
      metrics.logStructured({ phase: "fetch_error", startLedger });
      return;
    }

    metrics.incPages(1);

    let folded = 0;
    let failures = 0;
    for (const ev of page.events) {
      try {
        const res = await fold(ev);
        if (res.ok) {
          folded++;
          metrics.incEventsFolded(1);
        } else {
          failures++;
          metrics.incFoldFailures(1);
          console.error(JSON.stringify({ level: "warn", msg: "fold failed", error: res.error, type: ev.type, ledger: ev.ledger, txHash: ev.txHash }));
        }
      } catch (e) {
        failures++;
        metrics.incFoldFailures(1);
        console.error(JSON.stringify({ level: "warn", msg: "fold threw", error: String(e), type: ev.type, ledger: ev.ledger }));
      }
    }

    // Advance cursor according to pagination contract:
    // - If nextToken !== null, keep lastLedger at the same scanned upper bound? Actually Page.lastLedger is inclusive
    //   upper scanned; for continuation the same range continues, so lastLedger for cursor tracks the page's lastLedger
    //   only when nextToken === null; otherwise we preserve the scanned range. Scaffold: store page.lastLedger always
    //   and nextToken verbatim — poller will reuse nextToken on next tick without bumping lastLedger.
    const nextCursor: Cursor = {
      lastLedger: page.lastLedger,
      nextToken: page.nextToken,
    };

    // If this was the final page (nextToken null) the next tick will start at lastLedger+1.
    // Checkpoint.
    this.cursor = nextCursor;
    metrics.setCurrentCursorLedger(nextCursor.lastLedger);
    if (page.nextToken === null) {
      // Only mark successful poll when a full range is exhausted (no more pages), or empty page
      metrics.setLastSuccessfulPollNow();
      health.markSuccessfulPoll(nextCursor);
    } else {
      // Still more pages — update cursor but not yet "successful" until range fully drained
      health.setCursor(nextCursor);
    }

    if (this.opts.saveCursor) {
      try {
        await this.opts.saveCursor(nextCursor);
      } catch (e) {
        console.error(JSON.stringify({ level: "warn", msg: "failed to save cursor", error: String(e) }));
      }
    }

    metrics.logStructured({
      phase: "tick",
      startLedger,
      lastLedger: page.lastLedger,
      nextToken: page.nextToken ? "<present>" : null,
      eventsInPage: page.events.length,
      folded,
      failures,
    });
  }
}
