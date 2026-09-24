/**
 * Placeholder `SorobanEventSource` — returns empty pages so the poller can
 * run without a live RPC during scaffold. Replace with a real implementation
 * that calls `soroban-rpc` `getEvents` and maps to {@link Page}.
 *
 * The interface contract is defined in `types.ts: SorobanEventSource`.
 * Any replacement must honour:
 *  - `lastLedger` inclusive semantics
 *  - opaque `nextToken` round-tripping
 *  - `events` sorted ascending by `(ledger, sequence)`
 */

import { Page, GetEventsParams, SorobanEventSource } from "./types.js";

export class StubSorobanEventSource implements SorobanEventSource {
  async getEvents(params: GetEventsParams): Promise<Page> {
    const end = params.endLedger ?? params.startLedger;
    return {
      events: [],
      nextToken: null,
      lastLedger: end,
    };
  }
}

/**
 * Example real implementation sketch (not wired by default):
 *
 * ```ts
 * export class RpcSorobanEventSource implements SorobanEventSource {
 *   constructor(private rpcUrl: string, private contractIds: string[]) {}
 *   async getEvents(params: GetEventsParams): Promise<Page> {
 *     const res = await fetch(this.rpcUrl, {
 *       method: "POST",
 *       headers: { "Content-Type": "application/json" },
 *       body: JSON.stringify({
 *         jsonrpc: "2.0", id: 1, method: "getEvents",
 *         params: {
 *           startLedger: params.startLedger,
 *           filters: [{ type: "contract", contractIds: this.contractIds, topicFilters: [] }],
 *           pagination: params.cursor ? { cursor: params.cursor, limit: params.limit ?? 100 }
 *                      : { limit: params.limit ?? 100 },
 *         }
 *       })
 *     });
 *     const json = await res.json();
 *     // Map json.result.events -> SorobanEvent[], json.result.latestLedger -> lastLedger
 *     // json.result.cursor -> nextToken
 *   }
 * }
 * ```
 */
