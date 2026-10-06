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
 *
 * Loud-by-default (issue #568): constructing this stub logs a one-time
 * warning. Before that, a worker booted against this stub "succeeded" —
 * clean start, healthy `/healthz`, zero log lines, zero indexed events —
 * indistinguishable from a broken RPC returning nothing. Now the intent is
 * stated once, at construction, and silence after that means "no events on
 * chain", not "the stub is still wired in".
 */

import { Page, GetEventsParams, SorobanEventSource } from "./types.js";

export class StubSorobanEventSource implements SorobanEventSource {
  /** One warning per construction — never per `getEvents` call. */
  constructor() {
    console.warn(
      JSON.stringify({
        level: "warn",
        msg: "using placeholder SorobanEventSource — no events will be indexed",
        stub: "StubSorobanEventSource",
        file: "indexer/src/indexer/eventSource.ts",
        hint: "replace with an RPC-backed SorobanEventSource to index real events",
      })
    );
  }

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
