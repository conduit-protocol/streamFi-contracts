/**
 * Minimal health state for `GET /healthz`.
 *
 * Mirrors health-check patterns already used in `stream-fi-app` and
 * `streamFi-sdk` CI (HTTP probe that returns 200 when the process is
 * indexing and 503 when stuck) so orchestrators (k8s, systemd, PM2) can
 * distinguish:
 *   - "started and indexing"
 *   - "started but stuck on the placeholder SorobanEventSource"
 *   - "crashed" (no response)
 *
 * `lastSuccessfulPollTimestamp` is wall-clock ms; `currentCursor` is the
 * poller's high-water mark (see `types.ts:Cursor`). A threshold (default
 * 5 min) decides degraded status — if no successful poll for that long the
 * endpoint returns 503 so k8s liveness fails open and can restart the pod.
 */

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
}

export const health = new Health();
