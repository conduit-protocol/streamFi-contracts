/// Extend the processor instance's storage TTL.
///
/// Mirrors `contracts/stream/src/ttl.rs`: every state-mutating call renews the
/// instance record while its remaining TTL is at or below `MIN_TTL_LEDGERS`
/// (`TTL_THRESHOLD`), pushing it out to `EXTEND_TTL_LEDGERS` (`TTL_EXTEND_TO`).
///
/// The processor is stateless — it stores no keys — but its *instance* entry
/// still lives on the ledger and can be archived like any other contract's.
/// Without this bump a deployed processor that goes quiet between payouts
/// would eventually be archived (the exposure `docs/security.md` Known
/// Limitation #1 tracks), and every later batch would need a restore first.
pub use drip_common::ttl::bump_instance as bump;
