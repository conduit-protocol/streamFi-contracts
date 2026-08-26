export interface ChainEvent {
  id: string
  ledger: number
  type: string
  fields: Record<string, unknown>
}

export interface EventPage {
  events: ChainEvent[]
  nextToken: string | null
  lastLedger: number
}

/**
 * Fetches pages of contract events starting from `token` (null means "from genesis").
 * Implementations wrap the actual chain RPC (e.g. Soroban's getEvents); this interface
 * is what the poller depends on so it can be tested against a fake without a live node.
 */
export interface EventSource {
  fetchPage(token: string | null, pageSize: number): Promise<EventPage>
}
