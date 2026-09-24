/**
 * Minimal in-memory counters for the indexer.
 *
 * Exposed two ways so "why hasn't the indexer caught up" is graphable:
 * 1) Structured JSON log lines every poll tick (stderr, one JSON per line).
 * 2) Prometheus text format at `GET /metrics` (scraped by a dashboard).
 *
 * Counters are process-local; a restart resets them to zero — the dashboard
 * should use `increase()` / `rate()` over them. For persistence across
 * restarts the poller also checkpoints `lastLedger` to Postgres / file.
 */

export interface CountersSnapshot {
  pages_processed: number;
  events_folded: number;
  fold_failures: number;
  last_successful_poll_timestamp: number | null; // unix ms, wall clock
  current_cursor_ledger: number | null;
}

class Metrics {
  private pagesProcessed = 0;
  private eventsFolded = 0;
  private foldFailures = 0;
  private lastSuccessfulPollTimestamp: number | null = null;
  private currentCursorLedger: number | null = null;

  incPages(count = 1): void {
    this.pagesProcessed += count;
  }

  incEventsFolded(count = 1): void {
    this.eventsFolded += count;
  }

  incFoldFailures(count = 1): void {
    this.foldFailures += count;
  }

  setLastSuccessfulPollNow(): void {
    this.lastSuccessfulPollTimestamp = Date.now();
  }

  setLastSuccessfulPollTimestamp(ts: number | null): void {
    this.lastSuccessfulPollTimestamp = ts;
  }

  setCurrentCursorLedger(ledger: number | null): void {
    this.currentCursorLedger = ledger;
  }

  snapshot(): CountersSnapshot {
    return {
      pages_processed: this.pagesProcessed,
      events_folded: this.eventsFolded,
      fold_failures: this.foldFailures,
      last_successful_poll_timestamp: this.lastSuccessfulPollTimestamp,
      current_cursor_ledger: this.currentCursorLedger,
    };
  }

  /**
   * Emit one structured log line (JSON) to stderr. Intended to be called
   * once per poll tick so the dashboard can graph `events_folded` rate
   * without scraping `/metrics`.
   */
  logStructured(context: Record<string, unknown> = {}): void {
    const snap = this.snapshot();
    const line = JSON.stringify({
      ts: new Date().toISOString(),
      level: "info",
      msg: "indexer_tick",
      ...snap,
      ...context,
    });
    // stderr so stdout stays free for other tooling
    console.error(line);
  }

  /** Prometheus exposition format for `GET /metrics`. */
  toPrometheus(): string {
    const s = this.snapshot();
    const lines: string[] = [];
    lines.push("# HELP indexer_pages_processed_total Total pages fetched from Soroban RPC.");
    lines.push("# TYPE indexer_pages_processed_total counter");
    lines.push(`indexer_pages_processed_total ${s.pages_processed}`);
    lines.push("# HELP indexer_events_folded_total Total events successfully folded into projections.");
    lines.push("# TYPE indexer_events_folded_total counter");
    lines.push(`indexer_events_folded_total ${s.events_folded}`);
    lines.push("# HELP indexer_fold_failures_total Total events that failed to fold (will be retried or DLQ'd).");
    lines.push("# TYPE indexer_fold_failures_total counter");
    lines.push(`indexer_fold_failures_total ${s.fold_failures}`);
    lines.push("# HELP indexer_last_successful_poll_timestamp_seconds Unix timestamp (seconds) of last successful poll. 0 if never.");
    lines.push("# TYPE indexer_last_successful_poll_timestamp_seconds gauge");
    const tsSec = s.last_successful_poll_timestamp ? Math.floor(s.last_successful_poll_timestamp / 1000) : 0;
    lines.push(`indexer_last_successful_poll_timestamp_seconds ${tsSec}`);
    lines.push("# HELP indexer_current_cursor_ledger Ledger of current cursor (lastLedger). -1 if unknown.");
    lines.push("# TYPE indexer_current_cursor_ledger gauge");
    lines.push(`indexer_current_cursor_ledger ${s.current_cursor_ledger ?? -1}`);
    return lines.join("\n") + "\n";
  }
}

/** Singleton — the poller and the HTTP server share the same counters. */
export const metrics = new Metrics();
