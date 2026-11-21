use soroban_sdk::{symbol_short, Address, BytesN, Env, Symbol};

/// Emergency-pause observability events.
///
/// The factory's `pause`/`unpause` are already idempotent — a redundant call
/// reverts with `AlreadyPaused`/`NotPaused` rather than silently no-op'ing.
/// These events close the remaining gap: they let off-chain infrastructure
/// (indexers, relayers) positively confirm that a state transition committed,
/// rather than having to infer it from a bare "ok" that an ambiguous or
/// rate-limited RPC response may have dropped or duplicated. A relayer that
/// retried after a lost response can reconcile against the emitted event
/// instead of re-issuing a call it can no longer distinguish as a no-op.
///
/// Publication and the `set_paused` storage write are part of the same Soroban
/// transaction, so either both commit or both roll back — an event is never
/// emitted for a transition that did not actually persist.
///
/// Emitted when the factory transitions from unpaused to paused.
///
/// Topics: `("paused", governor)` — the governor address that authorized the halt.
/// Data:   `paused_at` — the ledger timestamp at which the halt took effect.
///
/// Note (Issue #628): Because all factory pause/unpause mutations are strictly
/// gated on `governor.require_auth()`, the `governor` topic identifies the governing
/// authority that authorized the halt. If the halt was triggered via
/// `DripGovernor::pause_factory`, the governor contract additionally emits
/// `("fpaused", caller)` containing the specific administrative caller (Role::Pauser/Admin).
pub fn paused(env: &Env, governor: &Address, paused_at: u64) {
    env.events()
        .publish((symbol_short!("paused"), governor.clone()), paused_at);
}

/// Emitted when the factory transitions from paused back to unpaused.
///
/// Topics: `("unpaused", governor)` — the governor address that lifted the halt.
/// Data:   `resumed_at` — the ledger timestamp at which creation resumed.
///
/// Note (Issue #628): Identifies the governing authority that lifted the halt.
/// If triggered via `DripGovernor::unpause_factory`, the governor contract
/// additionally emits `("funpaused", caller)` containing the specific administrative caller.
pub fn unpaused(env: &Env, governor: &Address, resumed_at: u64) {
    env.events()
        .publish((symbol_short!("unpaused"), governor.clone()), resumed_at);
}

pub fn upgraded(env: &Env, governor: &Address, upgraded_at: u64) {
    env.events()
        .publish((symbol_short!("upgraded"), governor.clone()), upgraded_at);
}

/// Emitted on every successfully created stream, once the governed protocol
/// fee has been routed to its recipient.
///
/// Topics: `("fee", fee_recipient)` — the address that received the fee.
/// Data:   `fee` — the amount routed (in stroops). Always the deposit's
///         `fee_bps / 10_000`, and `0` when the deposit is small enough that
///         the basis-point rounding yields nothing (or `fee_bps` is 0).
pub fn protocol_fee_charged(env: &Env, fee_recipient: &Address, fee: i128) {
    env.events()
        .publish((symbol_short!("fee"), fee_recipient.clone()), fee);
}

/// Protocol fee collection event (Issue #769).
///
/// Emitted when protocol fees are collected and routed to the recipient.
/// Allows indexers to track fee accumulation without scanning balance changes.
///
/// Topics: `("pfee_collected", recipient)` — the address that received the collected fee.
/// Data:   `(token, amount)` — the token address and amount collected (in stroops).
pub fn protocol_fee_collected(env: &Env, token: &Address, recipient: &Address, amount: i128) {
    env.events().publish(
        (Symbol::new(env, "pfee_collected"), recipient.clone()),
        (token.clone(), amount),
    );
}

/// StreamCreated event payload metadata (Issue #684).
#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamCreatedPayload {
    pub creator: Address,
    pub recipient: Address,
    pub stream_address: Address,
    pub salt: soroban_sdk::BytesN<32>,
    pub wasm_hash: soroban_sdk::BytesN<32>,
}

/// Emitted when a new stream is deployed and created by the factory (Issue #684).
///
/// Includes creator, recipient, stream address, salt, and wasm_hash in the event payload.
pub fn stream_created(
    env: &Env,
    creator: &Address,
    recipient: &Address,
    stream_address: &Address,
    salt: &soroban_sdk::BytesN<32>,
    wasm_hash: &soroban_sdk::BytesN<32>,
) {
    let payload = StreamCreatedPayload {
        creator: creator.clone(),
        recipient: recipient.clone(),
        stream_address: stream_address.clone(),
        salt: salt.clone(),
        wasm_hash: wasm_hash.clone(),
    };
    env.events().publish(
        (
            soroban_sdk::Symbol::new(env, "StreamCreated"),
            creator.clone(),
            recipient.clone(),
        ),
        payload,
    );
}

/// Emitted when the hash used for future stream deployments changes.
pub fn stream_contract_upgraded(env: &Env, old_wasm_hash: &BytesN<32>, new_wasm_hash: &BytesN<32>) {
    env.events().publish(
        (Symbol::new(env, "StreamContractUpgraded"),),
        (old_wasm_hash.clone(), new_wasm_hash.clone()),
    );
}

/// Emitted when a protocol fee update is proposed under timelock (Issue #766).
pub fn fee_update_proposed(env: &Env, proposed_fee_bps: u32, eta_ledger: u32) {
    env.events().publish(
        (Symbol::new(env, "fee_proposed"),),
        (proposed_fee_bps, eta_ledger),
    );
}

/// Emitted when a timelocked protocol fee update is executed (Issue #766).
pub fn fee_update_executed(env: &Env, old_fee_bps: u32, new_fee_bps: u32) {
    env.events().publish(
        (Symbol::new(env, "fee_executed"),),
        (old_fee_bps, new_fee_bps),
    );
}

/// Emitted when a protocol fee collector update is proposed under timelock (Issue #905).
pub fn fee_collector_update_proposed(env: &Env, proposed_collector: &Address, eta_ledger: u32) {
    env.events().publish(
        (Symbol::new(env, "fee_collector_proposed"),),
        (proposed_collector.clone(), eta_ledger),
    );
}

/// Emitted when a timelocked protocol fee collector update is executed (Issue #905).
pub fn fee_collector_update_executed(env: &Env, old_collector: &Address, new_collector: &Address) {
    env.events().publish(
        (Symbol::new(env, "fee_collector_executed"),),
        (old_collector.clone(), new_collector.clone()),
    );
}

/// Deprecation notice for a stream WASM version superseded by a governance
/// upgrade (Issue #902). Published once per superseded version, alongside
/// `StreamContractUpgraded`.
///
/// Topics: `("WasmVersionDeprecated",)`
/// Data:   `(version_number, wasm_hash, superseded_by_version)`
pub fn wasm_version_deprecated(
    env: &Env,
    version_number: u32,
    wasm_hash: &BytesN<32>,
    superseded_by_version: u32,
) {
    env.events().publish(
        (Symbol::new(env, "WasmVersionDeprecated"),),
        (version_number, wasm_hash.clone(), superseded_by_version),
    );
}

/// Emitted when an existing stream is migrated from one contract address to
/// another (Issue #903). Gives indexers an unambiguous link between the old and
/// new addresses and the migration time.
///
/// Topics: `("StreamMigrated",)`
/// Data:   `(old_address, new_address, migration_timestamp)`
///
/// No migration entrypoint currently calls this. `upgrade_stream_wasm` changes
/// only the hash used for future deployments and does not know any existing
/// stream address, and `upgrade_self` replaces the factory's own code. Wire it
/// in once a per-stream migration path exists.
#[allow(dead_code)]
pub fn stream_migrated(
    env: &Env,
    old_address: &Address,
    new_address: &Address,
    migration_timestamp: u64,
) {
    env.events().publish(
        (Symbol::new(env, "StreamMigrated"),),
        (old_address.clone(), new_address.clone(), migration_timestamp),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DripFactory;
    use soroban_sdk::testutils::{Address as _, Events as _};

    #[test]
    fn stream_migrated_publishes_exactly_one_event() {
        let env = Env::default();
        let contract_id = env.register_contract(None, DripFactory);
        let old_address = Address::generate(&env);
        let new_address = Address::generate(&env);

        env.as_contract(&contract_id, || {
            stream_migrated(&env, &old_address, &new_address, 1_700_000_000);
        });

        assert_eq!(env.events().all().len(), 1);
    }

    #[test]
    fn wasm_version_deprecated_publishes_exactly_one_event() {
        let env = Env::default();
        let contract_id = env.register_contract(None, DripFactory);
        let hash = BytesN::from_array(&env, &[3u8; 32]);

        env.as_contract(&contract_id, || {
            wasm_version_deprecated(&env, 1, &hash, 2);
        });

        assert_eq!(env.events().all().len(), 1);
    }
}
