use soroban_sdk::Env;

use drip_common::{TTL_EXTEND_TO, TTL_THRESHOLD};

/// Extend the processor instance's storage TTL.
///
/// Mirrors `contracts/stream/src/ttl.rs`: every state-mutating call renews the
/// instance record while its remaining TTL is at or below `TTL_THRESHOLD`,
/// pushing it out to `TTL_EXTEND_TO` (≈11.6 days, with a ≈5.8 day margin).
///
/// The processor is stateless — it stores no keys — but its *instance* entry
/// still lives on the ledger and can be archived like any other contract's.
/// Without this bump a deployed processor that goes quiet between payouts
/// would eventually be archived (the exposure `docs/security.md` Known
/// Limitation #1 tracks), and every later batch would need a restore first.
pub fn bump(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}
