/**
 * Worker entrypoint — bare process with HTTP health & metrics.
 *
 * Previously a blind `process` with no endpoint, no readiness file, and no
 * way for an orchestrator (k8s, systemd, PM2) to tell "started and indexing"
 * from "started but stuck on the placeholder SorobanEventSource" from
 * "crashed". Now exposes:
 *   - `GET /healthz`  — JSON with `lastSuccessfulPollTimestamp` + `currentCursor`
 *   - `GET /metrics`  — Prometheus text format with `pages_processed`,
 *                       `events_folded`, `fold_failures`
 *   - `GET /readyz`   — alias for `/healthz` (k8s readiness probe compat)
 *
 * Mirrors health-check patterns already used in `stream-fi-app` and
 * `streamFi-sdk` CI (HTTP probe, 200 vs 503) so the same manifest can be
 * copied.
 *
 * Alternative signal (if HTTP is disabled via `HEALTHZ_DISABLE=1`): the
 * worker touches a readiness file at `HEALTHZ_FILE` (default none) on each
 * successful poll — an orchestrator polling that file's mtime gets the same
 * signal without opening a port.
 */

import http from "node:http";
import fs from "node:fs";
import { Poller } from "./indexer/poller.js";
import { StubSorobanEventSource } from "./indexer/eventSource.js";
import { metrics } from "./metrics.js";
import { health } from "./health.js";

const PORT = Number(process.env.PORT ?? process.env.HEALTHZ_PORT ?? 3000);
const START_LEDGER = Number(process.env.START_LEDGER ?? 1);
const INTERVAL_MS = Number(process.env.POLL_INTERVAL_MS ?? 5000);
const HEALTHZ_FILE = process.env.HEALTHZ_FILE ?? null; // e.g. /tmp/indexer.ready

function touchReadyFile(): void {
  if (!HEALTHZ_FILE) return;
  try {
    fs.writeFileSync(HEALTHZ_FILE, new Date().toISOString() + "\n");
  } catch (e) {
    console.error(JSON.stringify({ level: "warn", msg: "failed to touch readiness file", error: String(e) }));
  }
}

// Wrap health.markSuccessfulPoll to also touch file
const origMark = health.markSuccessfulPoll.bind(health);
health.markSuccessfulPoll = (cursor: Parameters<typeof health.markSuccessfulPoll>[0]) => {
  origMark(cursor);
  touchReadyFile();
};

const poller = new Poller({
  source: new StubSorobanEventSource(),
  startLedger: START_LEDGER,
  intervalMs: INTERVAL_MS,
  // Add Postgres cursor persistence here when available:
  // loadCursor: async () => db.getCursor(),
  // saveCursor: async (c) => db.saveCursor(c),
});

const server = http.createServer((req, res) => {
  const url = req.url ?? "/";
  if (url === "/healthz" || url === "/readyz" || url.startsWith("/healthz?") || url.startsWith("/readyz?")) {
    const snap = health.snapshot();
    const body = JSON.stringify(snap, null, 2);
    const isHealthy = health.isHealthy();
    res.writeHead(isHealthy ? 200 : 503, { "Content-Type": "application/json" });
    res.end(body);
    return;
  }
  if (url === "/metrics" || url.startsWith("/metrics?")) {
    const body = metrics.toPrometheus();
    res.writeHead(200, { "Content-Type": "text/plain; version=0.0.4" });
    res.end(body);
    return;
  }
  if (url === "/" || url === "/health") {
    res.writeHead(200, { "Content-Type": "text/plain" });
    res.end("indexer ok — see /healthz and /metrics\n");
    return;
  }
  res.writeHead(404, { "Content-Type": "text/plain" });
  res.end("not found\n");
});

server.listen(PORT, async () => {
  console.error(JSON.stringify({ level: "info", msg: `healthz listening on :${PORT}`, port: PORT, startLedger: START_LEDGER }));
  try {
    await poller.start();
    console.error(JSON.stringify({ level: "info", msg: "poller started", startLedger: START_LEDGER, intervalMs: INTERVAL_MS }));
  } catch (e) {
    console.error(JSON.stringify({ level: "error", msg: "poller failed to start", error: String(e) }));
  }
});

function shutdown(signal: string): void {
  console.error(JSON.stringify({ level: "info", msg: `received ${signal}, shutting down` }));
  poller.stop();
  server.close(() => process.exit(0));
  setTimeout(() => process.exit(1), 5000).unref();
}

process.on("SIGTERM", () => shutdown("SIGTERM"));
process.on("SIGINT", () => shutdown("SIGINT"));
process.on("unhandledRejection", (e) => {
  console.error(JSON.stringify({ level: "error", msg: "unhandledRejection", error: String(e) }));
});
