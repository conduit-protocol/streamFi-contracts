/**
 * Shim for orchestrator health-check path `src/worker.ts`.
 *
 * Canonical implementation lives at `indexer/src/worker.ts` — this file
 * re-exports it so checks expecting `src/worker.ts` (as noted in the
 * scaffold review) still find a health endpoint at `GET /healthz` reporting
 * `lastSuccessfulPollTimestamp` and `currentCursor`.
 *
 * @see indexer/src/worker.ts
 * @see indexer/src/health.ts
 * @see indexer/src/metrics.ts
 */

export * from "../indexer/src/worker.js";

// When run directly via `npx tsx src/worker.ts` delegate to the canonical worker.
import "../indexer/src/worker.js";
