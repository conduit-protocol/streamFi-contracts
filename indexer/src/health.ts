/**
 * Minimal health state for `GET /health` and Prometheus metrics for `GET /metrics`.
 *
 * Mirrors health-check patterns already used in `stream-fi-app` and
 * `streamFi-sdk` CI (HTTP probe that returns 200 when the process is
 * indexing and 503 when stuck) so orchestrators (k8s, systemd, PM2) can
 * distinguish:
 *   - "started and indexing"
 *   - "started but stuck on the placeholder SorobanEventSource"
 *   - "crashed" (no response)
 *
 * Exposes Prometheus-compatible metrics endpoint at `GET /metrics` (Issue #701):
 *   - `indexer_last_processed_ledger`
 *   - `indexer_event_count_total`
 *   - `indexer_lag_ledgers`
 */

import http from "node:http";
import { Cursor } from "./indexer/types.js";

const STALE_THRESHOLD_MS = Number(process.env.HEALTHZ_STALE_MS ?? 5 * 60 * 1000);

export interface HealthSnapshot {
  status: "ok" | "degraded";
  lastSuccessfulPollTimestamp: number | null; // ms since epoch
  lastSuccessfulPollIso: string | null;
  currentCursor: Cursor | null;
  uptimeSeconds: number;
  version: string | null;
}

class Health {
  private lastSuccessfulPollTimestamp: number | null = null;
  private currentCursor: Cursor | null = null;
  private startedAt = Date.now();
  private version: string | null = null;
  private totalEventCount = 0;
  private latestLedger = 0;

  constructor() {
    try {
      // best-effort: read version from package.json if available
      // eslint-disable-next-line @typescript-eslint/no-var-requires
      const pkg = require("../../package.json");
      this.version = pkg.version ?? null;
    } catch {
      this.version = process.env.npm_package_version ?? null;
    }
  }

  recordEvents(count: number): void {
    this.totalEventCount += count;
  }

  setLatestLedger(ledger: number): void {
    this.latestLedger = ledger;
  }

  markSuccessfulPoll(cursor: Cursor): void {
    this.lastSuccessfulPollTimestamp = Date.now();
    this.currentCursor = { ...cursor };
  }

  setCursor(cursor: Cursor | null): void {
    this.currentCursor = cursor ? { ...cursor } : null;
  }

  snapshot(): HealthSnapshot {
    const now = Date.now();
    const stale = this.lastSuccessfulPollTimestamp !== null && now - this.lastSuccessfulPollTimestamp > STALE_THRESHOLD_MS;
    // Never polled yet but started < threshold ago -> ok (startup grace)
    const neverPolledButFresh = this.lastSuccessfulPollTimestamp === null && now - this.startedAt < STALE_THRESHOLD_MS;
    const status: HealthSnapshot["status"] = stale ? "degraded" : "ok";
    // If never polled and stale, still degraded
    const finalStatus = neverPolledButFresh ? "ok" : status;
    // If never polled and not fresh (long uptime), degraded
    const final = this.lastSuccessfulPollTimestamp === null && now - this.startedAt >= STALE_THRESHOLD_MS ? "degraded" : finalStatus;
    return {
      status: final,
      lastSuccessfulPollTimestamp: this.lastSuccessfulPollTimestamp,
      lastSuccessfulPollIso: this.lastSuccessfulPollTimestamp ? new Date(this.lastSuccessfulPollTimestamp).toISOString() : null,
      currentCursor: this.currentCursor,
      uptimeSeconds: Math.floor((now - this.startedAt) / 1000),
      version: this.version,
    };
  }

  isHealthy(): boolean {
    return this.snapshot().status === "ok";
  }

  /**
   * Generates Prometheus exposition text format for scrape targets (Issue #701).
   * Exports:
   * - indexer_last_processed_ledger
   * - indexer_event_count_total
   * - indexer_lag_ledgers
   * - indexer_uptime_seconds
   */
  getPrometheusMetrics(): string {
    const snap = this.snapshot();
    const lastProcessedLedger = snap.currentCursor?.lastLedger ?? 0;
    const lagLedgers = this.latestLedger > lastProcessedLedger ? this.latestLedger - lastProcessedLedger : 0;
    const lines: string[] = [
      "# HELP indexer_last_processed_ledger Latest ledger number processed by the indexer",
      "# TYPE indexer_last_processed_ledger gauge",
      `indexer_last_processed_ledger ${lastProcessedLedger}`,
      "# HELP indexer_event_count_total Total count of events processed by the indexer",
      "# TYPE indexer_event_count_total counter",
      `indexer_event_count_total ${this.totalEventCount}`,
      "# HELP indexer_lag_ledgers Difference between latest chain ledger and indexer cursor",
      "# TYPE indexer_lag_ledgers gauge",
      `indexer_lag_ledgers ${lagLedgers}`,
      "# HELP indexer_uptime_seconds Process uptime in seconds",
      "# TYPE indexer_uptime_seconds gauge",
      `indexer_uptime_seconds ${snap.uptimeSeconds}`,
    ];
    return lines.join("\n") + "\n";
  }
}

export const health = new Health();

/**
 * Handle incoming HTTP requests for health check and Prometheus scrape endpoints.
 * Returns true if request was handled, false otherwise.
 */
export function handleHealthRequest(req: http.IncomingMessage, res: http.ServerResponse): boolean {
  const url = req.url ?? "/";
  if (url === "/health" || url === "/healthz" || url === "/readyz" || url.startsWith("/health?") || url.startsWith("/healthz?") || url.startsWith("/readyz?")) {
    const snap = health.snapshot();
    const isHealthy = health.isHealthy();
    res.writeHead(isHealthy ? 200 : 503, { "Content-Type": "application/json" });
    res.end(JSON.stringify(snap, null, 2));
    return true;
  }
  if (url === "/metrics" || url.startsWith("/metrics?")) {
    const body = health.getPrometheusMetrics();
    res.writeHead(200, { "Content-Type": "text/plain; version=0.0.4; charset=utf-8" });
    res.end(body);
    return true;
  }
  return false;
}

/**
 * Create a standalone HTTP server for health and Prometheus scrape metrics.
 */
export function createHealthServer(port = Number(process.env.PORT ?? process.env.HEALTHZ_PORT ?? 3000)): http.Server {
  const server = http.createServer((req, res) => {
    if (handleHealthRequest(req, res)) {
      return;
    }
    if (req.url === "/" || req.url === "") {
      res.writeHead(200, { "Content-Type": "text/plain" });
      res.end("indexer ok — see /health and /metrics\n");
      return;
    }
    res.writeHead(404, { "Content-Type": "text/plain" });
    res.end("not found\n");
  });
  return server;
}
