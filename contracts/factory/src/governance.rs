use drip_governor::DripGovernorClient;
pub use drip_governor::GovernorConfig;
use soroban_sdk::{Address, BytesN, Env, Vec};

use crate::storage::DataKey;
use crate::{events, Error};

/// One entry in the stream WASM version history (Issue #902).
///
/// Version numbers start at 1 and increase by one per governance-driven change
/// of the stream WASM hash.
#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WasmVersionInfo {
    /// One-based, monotonically increasing version number.
    pub version_number: u32,
    /// WASM hash that became the stream deployment hash at `deployed_at_ledger`.
    pub wasm_hash: BytesN<32>,
    /// Ledger sequence at which this hash was set. `0` marks a legacy entry
    /// (a factory initialised before version history was recorded) whose
    /// deployment ledger is unknown.
    pub deployed_at_ledger: u32,
}

/// Records the first stream WASM version when the factory is initialised (Issue #902).
pub fn record_initial_wasm_version(env: &Env, wasm_hash: &BytesN<32>) {
    let mut history: Vec<WasmVersionInfo> = Vec::new(env);
    history.push_back(WasmVersionInfo {
        version_number: 1,
        wasm_hash: wasm_hash.clone(),
        deployed_at_ledger: env.ledger().sequence(),
    });
    env.storage()
        .instance()
        .set(&DataKey::WasmVersionHistory, &history);
}

/// Records a governance-driven change of the stream WASM hash (Issue #902).
///
/// Appends the new hash with the current ledger, then publishes one deprecation
/// notice for the version it supersedes. A factory initialised before version
/// history existed has no recorded versions, so the superseded hash is
/// backfilled as version 1 with `deployed_at_ledger = 0` before the new entry is
/// appended.
///
/// Callers must invoke this only when the hash actually changes; re-setting the
/// current hash records nothing.
pub fn record_wasm_version_change(
    env: &Env,
    old_wasm_hash: &BytesN<32>,
    new_wasm_hash: &BytesN<32>,
) {
    let mut history: Vec<WasmVersionInfo> = env
        .storage()
        .instance()
        .get(&DataKey::WasmVersionHistory)
        .unwrap_or(Vec::new(env));

    if history.is_empty() {
        history.push_back(WasmVersionInfo {
            version_number: 1,
            wasm_hash: old_wasm_hash.clone(),
            deployed_at_ledger: 0,
        });
    }

    let superseded = history.get(history.len() - 1).unwrap();
    let new_version_number = superseded.version_number.saturating_add(1);
    history.push_back(WasmVersionInfo {
        version_number: new_version_number,
        wasm_hash: new_wasm_hash.clone(),
        deployed_at_ledger: env.ledger().sequence(),
    });
    env.storage()
        .instance()
        .set(&DataKey::WasmVersionHistory, &history);

    events::wasm_version_deprecated(
        env,
        superseded.version_number,
        &superseded.wasm_hash,
        new_version_number,
    );
}

/// Full stream WASM version history, oldest first (Issue #902).
///
/// The last entry matches the hash currently used for deployments. Returns an
/// empty vector only for a factory whose history has never been recorded and
/// that has not had its hash changed since.
pub fn wasm_version_history(env: &Env) -> Vec<WasmVersionInfo> {
    env.storage()
        .instance()
        .get(&DataKey::WasmVersionHistory)
        .unwrap_or(Vec::new(env))
}

/// Fetches the live protocol config from `governor` via a cross-contract call.
///
/// Returns `Err(GovernorNotResponding)` when the cross-contract call fails
/// (governor archived / not initialised / host error) instead of letting the
/// host trap bubble up as an opaque error.
pub fn config(env: &Env, governor: &Address) -> Result<GovernorConfig, Error> {
    // Cross-contract call — flattens the nested Result from try_config()
    // (outer = host error, inner = governor contract error) so callers see
    // a meaningful `GovernorNotResponding` instead of an opaque host trap.
    let result = DripGovernorClient::new(env, governor)
        .try_config()
        .map_err(|_| Error::GovernorNotResponding)?;
    result.map_err(|_| Error::GovernorNotResponding)
}

/// Enforces the governor-controlled rate/duration bounds on a new stream.
///
/// `rate_per_sec` and, for fixed-duration streams, the declared length must
/// respect the protocol parameters DripGovernor holds.
///
/// # Design Note: Creation vs Extension Bounds
///
/// This duration bound is enforced at stream creation time to cap initial upfront
/// commitment and scheduling horizons. Post-creation lifetime extensions via
/// `DripStream::extend_duration` and `DripStream::top_up_and_extend` are
/// intentionally unbounded by this initial creation cap, allowing ongoing streams
/// (e.g. payroll, recurring subscriptions) to continue without redeploying per ADR-001.
///
/// # Errors
///
/// This function uses `checked_sub` to safely compute stream duration, ensuring
/// it is robust against any call site — including future code paths that may
/// not pre-validate `end_time > start_time`. Returns `Err(ArithmeticOverflow)`
/// if `end_time < start_time` (underflow).
pub fn enforce_bounds(
    config: &GovernorConfig,
    rate_per_sec: i128,
    start_time: u64,
    end_time: u64,
    now: u64,
) -> Result<(), Error> {
    if rate_per_sec > config.max_rate_per_second {
        return Err(Error::RateExceedsMax);
    }

    // Bound how far ahead a stream may be scheduled.
    //
    // Without this, `start_time = now + 100 years` with `end_time = 0` is
    // accepted: the duration checks below only run for fixed-duration streams,
    // so an open-ended stream escapes every bound. The deposit is transferred
    // immediately and, when clawback is disabled, cannot be recovered before
    // `start_time` — which is to say, never in practice.
    //
    // `max_duration_seconds` is reused as the ceiling rather than introducing a
    // separate governor parameter: a stream that cannot begin for longer than
    // the protocol's longest permitted stream is locking funds beyond anything
    // a legitimate stream would. Reusing it also keeps this fix inside the
    // factory instead of migrating the governor's stored config.
    let start_offset = start_time.saturating_sub(now);
    if start_offset > config.max_duration_seconds {
        return Err(Error::StartTimeTooFarInFuture);
    }

    if end_time > 0 {
        let duration = end_time
            .checked_sub(start_time)
            .ok_or(Error::ArithmeticOverflow)?;
        if duration < config.min_duration_seconds {
            return Err(Error::DurationTooShort);
        }
        if duration > config.max_duration_seconds {
            return Err(Error::DurationExceedsMax);
        }
    }
    Ok(())
}

/// Minimum timelock delay required before a proposed fee update can be executed (Issue #766).
pub const MIN_DELAY: u32 = 17_280; // ~1 day on Soroban at 5s/ledger

#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeUpdateProposal {
    pub proposed_fee_bps: u32,
    pub proposed_at_ledger: u32,
    pub eta_ledger: u32,
}

/// Proposal for updating the protocol fee collector address subject to timelock (Issue #905).
#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeCollectorUpdateProposal {
    pub proposed_collector: Address,
    pub proposed_at_ledger: u32,
    pub eta_ledger: u32,
}

/// Proposes a protocol fee update subject to a timelock delay of at least `MIN_DELAY` ledgers (Issue #766).
pub fn propose_fee_update(env: &Env, proposed_fee_bps: u32) -> Result<u32, Error> {
    if proposed_fee_bps > 10_000 {
        return Err(Error::InvalidFee);
    }
    let current_ledger = env.ledger().sequence();
    let eta_ledger = current_ledger.saturating_add(MIN_DELAY);
    let proposal = FeeUpdateProposal {
        proposed_fee_bps,
        proposed_at_ledger: current_ledger,
        eta_ledger,
    };
    env.storage()
        .instance()
        .set(&crate::storage::DataKey::PendingFeeUpdate, &proposal);
    crate::events::fee_update_proposed(env, proposed_fee_bps, eta_ledger);
    Ok(eta_ledger)
}

/// Executes a pending protocol fee update once `MIN_DELAY` ledgers have elapsed (Issue #766).
pub fn execute_fee_update(env: &Env) -> Result<(), Error> {
    let proposal: FeeUpdateProposal = env
        .storage()
        .instance()
        .get(&crate::storage::DataKey::PendingFeeUpdate)
        .ok_or(Error::NoPendingFeeUpdate)?;

    let current_ledger = env.ledger().sequence();
    if current_ledger < proposal.eta_ledger {
        return Err(Error::TimelockNotMet);
    }

    let old_fee_bps = env
        .storage()
        .instance()
        .get(&crate::storage::DataKey::ProtocolFeeBps)
        .unwrap_or(30);

    env.storage()
        .instance()
        .set(&crate::storage::DataKey::ProtocolFeeBps, &proposal.proposed_fee_bps);
    env.storage()
        .instance()
        .remove(&crate::storage::DataKey::PendingFeeUpdate);

    crate::events::fee_update_executed(env, old_fee_bps, proposal.proposed_fee_bps);
    Ok(())
}

/// Read pending fee update proposal, if any (Issue #766).
pub fn pending_fee_update(env: &Env) -> Option<FeeUpdateProposal> {
    env.storage()
        .instance()
        .get(&crate::storage::DataKey::PendingFeeUpdate)
}

/// Proposes a protocol fee collector update subject to a timelock delay of at least `MIN_DELAY` ledgers (Issue #905).
pub fn propose_fee_collector_update(env: &Env, proposed_collector: &Address) -> Result<u32, Error> {
    let current_ledger = env.ledger().sequence();
    let eta_ledger = current_ledger.saturating_add(MIN_DELAY);
    let proposal = FeeCollectorUpdateProposal {
        proposed_collector: proposed_collector.clone(),
        proposed_at_ledger: current_ledger,
        eta_ledger,
    };
    env.storage()
        .instance()
        .set(&crate::storage::DataKey::PendingFeeCollectorUpdate, &proposal);
    crate::events::fee_collector_update_proposed(env, proposed_collector, eta_ledger);
    Ok(eta_ledger)
}

/// Executes a pending protocol fee collector update once `MIN_DELAY` ledgers have elapsed (Issue #905).
pub fn execute_fee_collector_update(env: &Env) -> Result<(), Error> {
    let proposal: FeeCollectorUpdateProposal = env
        .storage()
        .instance()
        .get(&crate::storage::DataKey::PendingFeeCollectorUpdate)
        .ok_or(Error::NoPendingFeeUpdate)?;

    let current_ledger = env.ledger().sequence();
    if current_ledger < proposal.eta_ledger {
        return Err(Error::TimelockNotMet);
    }

    let old_collector = env
        .storage()
        .instance()
        .get::<_, Address>(&crate::storage::DataKey::FeeCollectorAddress)
        .ok_or(Error::NotInitialized)?;

    env.storage()
        .instance()
        .set(&crate::storage::DataKey::FeeCollectorAddress, &proposal.proposed_collector);
    env.storage()
        .instance()
        .remove(&crate::storage::DataKey::PendingFeeCollectorUpdate);

    crate::events::fee_collector_update_executed(env, &old_collector, &proposal.proposed_collector);
    Ok(())
}

/// Read pending fee collector update proposal, if any (Issue #905).
pub fn pending_fee_collector_update(env: &Env) -> Option<FeeCollectorUpdateProposal> {
    env.storage()
        .instance()
        .get(&crate::storage::DataKey::PendingFeeCollectorUpdate)
}

/// Read active protocol fee override if set via timelocked execution.
pub fn active_fee_bps(env: &Env) -> Option<u32> {
    env.storage()
        .instance()
        .get(&crate::storage::DataKey::ProtocolFeeBps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    const HOUR: u64 = 3_600;
    const DAY: u64 = 86_400;
    const NOW: u64 = 1_700_000_000;

    fn config(env: &Env) -> GovernorConfig {
        GovernorConfig {
            fee_bps: 30,
            fee_recipient: Address::generate(env),
            min_duration_seconds: HOUR,
            max_duration_seconds: 30 * DAY,
            max_rate_per_second: 1_000_000,
            factory_address: Address::generate(env),
            force_cancel_pause_secs: 30 * DAY,
            min_quorum_bps: 0,
            voting_period_ledgers: 17_280,
            proposal_stake_token: None,
            min_proposal_stake: 0,
        }
    }

    // ── start_time upper bound (#342) ───────────────────────────────────────
    //
    // `create_stream` rejects a backdated start, but nothing bounded how far
    // ahead one could be. With `end_time = 0` the duration checks below never
    // run, so `start_time = now + 100 years` was accepted: the deposit moves
    // immediately and, with clawback disabled, cannot be recovered before the
    // stream starts.

    #[test]
    fn open_ended_stream_far_in_the_future_is_rejected() {
        let env = Env::default();
        let cfg = config(&env);

        // 100 years out, open-ended — the exact case from the issue.
        let start = NOW + 100 * 365 * DAY;

        assert_eq!(
            enforce_bounds(&cfg, 1, start, 0, NOW),
            Err(Error::StartTimeTooFarInFuture)
        );
    }

    #[test]
    fn fixed_duration_stream_far_in_the_future_is_also_rejected() {
        // The bound applies regardless of end_time; a fixed-duration stream
        // scheduled beyond the ceiling locks funds just as long.
        let env = Env::default();
        let cfg = config(&env);
        let start = NOW + 100 * 365 * DAY;

        assert_eq!(
            enforce_bounds(&cfg, 1, start, start + DAY, NOW),
            Err(Error::StartTimeTooFarInFuture)
        );
    }

    #[test]
    fn start_exactly_at_the_ceiling_is_allowed() {
        // The bound is inclusive: an offset equal to max_duration_seconds is
        // the largest legitimate schedule, not one past it.
        let env = Env::default();
        let cfg = config(&env);
        let start = NOW + cfg.max_duration_seconds;

        assert_eq!(enforce_bounds(&cfg, 1, start, 0, NOW), Ok(()));
    }

    #[test]
    fn start_one_second_past_the_ceiling_is_rejected() {
        let env = Env::default();
        let cfg = config(&env);
        let start = NOW + cfg.max_duration_seconds + 1;

        assert_eq!(
            enforce_bounds(&cfg, 1, start, 0, NOW),
            Err(Error::StartTimeTooFarInFuture)
        );
    }

    #[test]
    fn immediate_start_is_unaffected() {
        // The overwhelmingly common case must not regress.
        let env = Env::default();
        let cfg = config(&env);

        assert_eq!(enforce_bounds(&cfg, 1, NOW, 0, NOW), Ok(()));
        assert_eq!(enforce_bounds(&cfg, 1, NOW, NOW + DAY, NOW), Ok(()));
    }

    #[test]
    fn a_start_time_already_in_the_past_does_not_underflow() {
        // `create_stream` rejects backdated starts before reaching here, but
        // enforce_bounds is also called from the batch path and must not panic
        // on a past timestamp. saturating_sub yields a zero offset.
        let env = Env::default();
        let cfg = config(&env);

        assert_eq!(enforce_bounds(&cfg, 1, NOW - 5 * DAY, 0, NOW), Ok(()));
    }

    // ── Existing bounds still hold ──────────────────────────────────────────

    #[test]
    fn rate_above_the_maximum_is_still_rejected() {
        let env = Env::default();
        let cfg = config(&env);

        assert_eq!(
            enforce_bounds(&cfg, cfg.max_rate_per_second + 1, NOW, 0, NOW),
            Err(Error::RateExceedsMax)
        );
    }

    #[test]
    fn duration_bounds_still_apply_to_fixed_duration_streams() {
        let env = Env::default();
        let cfg = config(&env);

        assert_eq!(
            enforce_bounds(&cfg, 1, NOW, NOW + cfg.min_duration_seconds - 1, NOW),
            Err(Error::DurationTooShort)
        );
        assert_eq!(
            enforce_bounds(&cfg, 1, NOW, NOW + cfg.max_duration_seconds + 1, NOW),
            Err(Error::DurationExceedsMax)
        );
    }

    // ── Stream WASM version history (Issue #902) ────────────────────────────

    use crate::{DripFactory, DripFactoryClient};
    use soroban_sdk::testutils::{Events as _, Ledger as _};

    /// Registers and initialises a factory with `[1; 32]` as its stream WASM hash.
    /// Auth is mocked so the governor-gated upgrade path authorises automatically.
    fn factory(env: &Env) -> (Address, DripFactoryClient<'static>) {
        env.mock_all_auths();
        let contract_id = env.register_contract(None, DripFactory);
        let client = DripFactoryClient::new(env, &contract_id);
        client.initialize(&BytesN::from_array(env, &[1u8; 32]), &Address::generate(env));
        (contract_id, client)
    }

    #[test]
    fn initialize_records_version_one_at_the_current_ledger() {
        let env = Env::default();
        env.ledger().set_sequence_number(42);
        let (_, client) = factory(&env);

        let history = client.wasm_version_history();

        assert_eq!(history.len(), 1);
        let v1 = history.get(0).unwrap();
        assert_eq!(v1.version_number, 1);
        assert_eq!(v1.wasm_hash, BytesN::from_array(&env, &[1u8; 32]));
        assert_eq!(v1.deployed_at_ledger, 42);
    }

    #[test]
    fn upgrade_appends_a_version_with_its_ledger_and_sets_the_hash() {
        let env = Env::default();
        let (_, client) = factory(&env);
        env.ledger().set_sequence_number(100);
        let new_hash = BytesN::from_array(&env, &[2u8; 32]);

        client.upgrade_stream_wasm(&new_hash);

        assert_eq!(client.stream_wasm_hash(), new_hash);
        let history = client.wasm_version_history();
        assert_eq!(history.len(), 2);
        let v2 = history.get(1).unwrap();
        assert_eq!(v2.version_number, 2);
        assert_eq!(v2.wasm_hash, new_hash);
        assert_eq!(v2.deployed_at_ledger, 100);
    }

    #[test]
    fn superseded_version_emits_a_deprecation_notice() {
        let env = Env::default();
        let (_, client) = factory(&env);
        let before = env.events().all().len();

        client.upgrade_stream_wasm(&BytesN::from_array(&env, &[2u8; 32]));

        // One `StreamContractUpgraded` plus one deprecation notice for version 1.
        assert_eq!(env.events().all().len(), before + 2);
    }

    #[test]
    fn re_setting_the_current_hash_records_nothing() {
        let env = Env::default();
        let (_, client) = factory(&env);
        let before_events = env.events().all().len();

        client.upgrade_stream_wasm(&BytesN::from_array(&env, &[1u8; 32]));

        assert_eq!(client.wasm_version_history().len(), 1);
        assert_eq!(env.events().all().len(), before_events);
    }

    #[test]
    fn rejected_zero_hash_leaves_history_unchanged() {
        let env = Env::default();
        let (_, client) = factory(&env);

        let result = client.try_upgrade_stream_wasm(&BytesN::from_array(&env, &[0u8; 32]));

        assert!(result.is_err());
        assert_eq!(client.wasm_version_history().len(), 1);
    }

    #[test]
    fn legacy_factory_without_history_is_backfilled_on_first_upgrade() {
        let env = Env::default();
        let (contract_id, client) = factory(&env);
        // Simulate a factory initialised before version history existed.
        env.as_contract(&contract_id, || {
            env.storage().instance().remove(&DataKey::WasmVersionHistory);
        });

        client.upgrade_stream_wasm(&BytesN::from_array(&env, &[2u8; 32]));

        let history = client.wasm_version_history();
        assert_eq!(history.len(), 2);
        let legacy = history.get(0).unwrap();
        assert_eq!(legacy.version_number, 1);
        assert_eq!(legacy.wasm_hash, BytesN::from_array(&env, &[1u8; 32]));
        assert_eq!(legacy.deployed_at_ledger, 0);
        assert_eq!(history.get(1).unwrap().version_number, 2);
    }

    #[test]
    fn end_before_start_still_reports_overflow() {
        let env = Env::default();
        let cfg = config(&env);

        assert_eq!(
            enforce_bounds(&cfg, 1, NOW + DAY, NOW, NOW),
            Err(Error::ArithmeticOverflow)
        );
    }
}
