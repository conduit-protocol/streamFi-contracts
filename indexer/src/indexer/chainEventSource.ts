import type { EventPage, EventSource } from './types.js'

/**
 * Placeholder EventSource. Not wired to a live Soroban RPC endpoint yet — this scaffold
 * only establishes the poller/worker/handler structure. Replace with a real
 * implementation that calls the RPC's event-streaming endpoint before running this
 * against a live network.
 */
export class SorobanEventSource implements EventSource {
  async fetchPage(_token: string | null, _pageSize: number): Promise<EventPage> {
    return { events: [], nextToken: _token, lastLedger: 0 }
  }
}
