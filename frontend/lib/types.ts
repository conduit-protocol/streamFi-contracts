/**
 * Shared domain types for wallet/network-facing components (`TokenSelector`,
 * `NetworkSwitcher`, ...). Kept here rather than inline in each component so
 * the shape doesn't drift as more components need the same `Token`/`Network`
 * concept.
 */

/** A token selectable in `TokenSelector` — a Soroban contract ID plus a display label. */
export interface Token {
  label: string;
  address: string;
}

/** A Stellar/Soroban network selectable in `NetworkSwitcher`. */
export interface Network {
  id: string;
  name: string;
  rpcUrl: string;
}
