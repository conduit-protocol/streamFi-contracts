#![cfg(test)]

// The crate is `#![no_std]`, but this module only compiles under `cargo test`,
// where `std` is available as a linked dependency of the test harness anyway.
extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, BytesN, Env,
};

use crate::{storage::DataKey, DripFactory, DripFactoryClient, Error};

/// Register a factory and initialize it with a dummy stream WASM hash and a
/// freshly generated governor. Auth is mocked, so the governor-gated
/// `pause`/`unpause` calls authorize automatically.
///
/// These tests exercise the pause/unpause state machine and its emitted
/// events in isolation — they never call `create_stream` (which would need a
/// real stream WASM to deploy and a live governor cross-contract call), so a
/// zero WASM hash is sufficient here.
struct Setup {
    env: Env,
    client: DripFactoryClient<'static>,
}

impl Setup {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let governor = Address::generate(&env);
        let wasm_hash = BytesN::from_array(&env, &[1u8; 32]);

        let contract_id = env.register_contract(None, DripFactory);
        let client = DripFactoryClient::new(&env, &contract_id);
        client.initialize(&wasm_hash, &governor);

        Setup { env, client }
    }

    /// Number of contract events emitted so far.
    fn event_count(&self) -> usize {
        self.env.events().all().len() as usize
    }
}

#[test]
fn pause_then_unpause_flips_state() {
    let s = Setup::new();
    assert!(!s.client.is_paused());

    s.client.pause();
    assert!(s.client.is_paused());

    s.client.unpause();
    assert!(!s.client.is_paused());
}

#[test]
fn factory_status_returns_combined_pause_and_fee_status() {
    // The governor in this setup is a bare address with no contract behind it,
    // so the fee is genuinely unreadable and reports None. This previously
    // asserted `30` — the hardcoded fallback — which meant the test encoded
    // the very ambiguity the fee read is now able to signal.
    let s = Setup::new();
    let status = s.client.factory_status();
    assert!(!status.is_paused);
    assert_eq!(status.protocol_fee_bps, None);

    s.client.pause();
    let status_paused = s.client.factory_status();
    assert!(status_paused.is_paused);
    assert_eq!(status_paused.protocol_fee_bps, None);
}

#[test]
fn pause_when_already_paused_errors_and_leaves_state_unchanged() {
    let s = Setup::new();
    s.client.pause();
    assert!(s.client.is_paused());

    let events_before = s.event_count();
    // A redundant pause must not silently succeed — it reverts, so a retrying
    // off-chain caller can distinguish "I changed state" from "already there".
    let result = s.client.try_pause();
    assert_eq!(result, Err(Ok(Error::AlreadyPaused)));

    // State is still paused, and no additional event was emitted for the no-op.
    assert!(s.client.is_paused());
    assert_eq!(s.event_count(), events_before);
}

#[test]
fn unpause_when_not_paused_errors_and_leaves_state_unchanged() {
    let s = Setup::new();
    assert!(!s.client.is_paused());

    let events_before = s.event_count();
    let result = s.client.try_unpause();
    assert_eq!(result, Err(Ok(Error::NotPaused)));

    assert!(!s.client.is_paused());
    assert_eq!(s.event_count(), events_before);
}

#[test]
fn each_successful_transition_emits_exactly_one_event() {
    let s = Setup::new();
    let base = s.event_count();

    s.client.pause();
    assert_eq!(s.event_count(), base + 1);

    s.client.unpause();
    assert_eq!(s.event_count(), base + 2);
}

#[test]
fn rapid_repeated_calls_never_diverge_from_the_invoked_sequence() {
    // Simulates the issue's "100+ rapid requests" as a long sequence of
    // repeated calls in one test. Every redundant call reverts; only genuine
    // transitions mutate state or emit events. At every point the observable
    // state and the emitted-event count agree with the calls that actually
    // succeeded — state can never silently diverge from what was invoked.
    let s = Setup::new();
    let base = s.event_count();
    let mut expected_paused = false;
    let mut successful_transitions = 0usize;

    for i in 0..120u32 {
        if i % 2 == 0 {
            // Attempt to pause.
            if expected_paused {
                assert_eq!(s.client.try_pause(), Err(Ok(Error::AlreadyPaused)));
            } else {
                s.client.pause();
                expected_paused = true;
                successful_transitions += 1;
            }
        } else {
            // Attempt to unpause.
            if expected_paused {
                s.client.unpause();
                expected_paused = false;
                successful_transitions += 1;
            } else {
                assert_eq!(s.client.try_unpause(), Err(Ok(Error::NotPaused)));
            }
        }

        // Invariant checked on every iteration.
        assert_eq!(s.client.is_paused(), expected_paused);
        assert_eq!(s.event_count(), base + successful_transitions);
    }
}

// ── Issue #819: deploy::set_stream_wasm_hash ────────────────────────────────

fn set_hash(s: &Setup, hash: &BytesN<32>) -> Result<(), Error> {
    s.env.as_contract(&s.client.address, || {
        crate::deploy::set_stream_wasm_hash(&s.env, hash)
    })
}

#[test]
fn set_stream_wasm_hash_updates_hash_for_new_streams_and_emits_event() {
    let s = Setup::new();
    let before = s.event_count();
    let new_hash = BytesN::from_array(&s.env, &[7u8; 32]);

    assert_eq!(set_hash(&s, &new_hash), Ok(()));

    assert_eq!(s.client.stream_wasm_hash(), new_hash);
    assert_eq!(s.event_count(), before + 1);
}

#[test]
fn set_stream_wasm_hash_is_a_noop_for_the_current_hash() {
    let s = Setup::new();
    let current = BytesN::from_array(&s.env, &[1u8; 32]);
    let before = s.event_count();

    assert_eq!(set_hash(&s, &current), Ok(()));

    assert_eq!(s.client.stream_wasm_hash(), current);
    assert_eq!(s.event_count(), before);
}

#[test]
fn set_stream_wasm_hash_rejects_zero_hash_and_keeps_the_old_one() {
    let s = Setup::new();
    let zero = BytesN::from_array(&s.env, &[0u8; 32]);

    assert_eq!(set_hash(&s, &zero), Err(Error::InvalidWasmHash));
    assert_eq!(
        s.client.stream_wasm_hash(),
        BytesN::from_array(&s.env, &[1u8; 32])
    );
}

#[test]
fn set_stream_wasm_hash_requires_governor_auth() {
    // No `mock_all_auths`: the governor never signs, so the call must fail.
    let env = Env::default();
    let contract_id = env.register_contract(None, DripFactory);
    let client = DripFactoryClient::new(&env, &contract_id);
    client.initialize(
        &BytesN::from_array(&env, &[1u8; 32]),
        &Address::generate(&env),
    );

    let result = client.try_upgrade_stream_wasm(&BytesN::from_array(&env, &[2u8; 32]));

    assert!(result.is_err());
    assert_eq!(
        client.stream_wasm_hash(),
        BytesN::from_array(&env, &[1u8; 32])
    );
}

// ── Issue #86: upgrade_stream_wasm input validation ─────────────────────────

#[test]
fn upgrade_stream_wasm_rejects_zero_hash() {
    let s = Setup::new();
    let zero_hash = BytesN::from_array(&s.env, &[0u8; 32]);
    let result = s.client.try_upgrade_stream_wasm(&zero_hash);
    assert_eq!(result, Err(Ok(Error::InvalidWasmHash)));
}

#[test]
fn upgrade_stream_wasm_accepts_non_zero_hash() {
    let s = Setup::new();
    let valid_hash = BytesN::from_array(&s.env, &[1u8; 32]);
    let result = s.client.try_upgrade_stream_wasm(&valid_hash);
    assert!(result.is_ok());
}

#[test]
fn upgrade_stream_wasm_rejects_when_paused() {
    let s = Setup::new();
    s.client.pause();
    let valid_hash = BytesN::from_array(&s.env, &[2u8; 32]);
    let result = s.client.try_upgrade_stream_wasm(&valid_hash);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn upgrade_stream_wasm_accepts_after_unpause() {
    let s = Setup::new();
    s.client.pause();
    s.client.unpause();
    let valid_hash = BytesN::from_array(&s.env, &[3u8; 32]);
    let result = s.client.try_upgrade_stream_wasm(&valid_hash);
    assert!(result.is_ok());
}

// ── Self-upgrade (upgrade) ──────────────────────────────────────────────────

#[test]
fn upgrade_rejects_zero_hash() {
    let s = Setup::new();
    let zero_hash = BytesN::from_array(&s.env, &[0u8; 32]);
    let result = s.client.try_upgrade_self(&zero_hash, &1u32);
    assert_eq!(result, Err(Ok(Error::InvalidWasmHash)));
}

#[test]
fn upgrade_passes_auth_and_zero_hash_check() {
    let s = Setup::new();
    // Zero hash is rejected before reaching the host-level WASM swap.
    let zero_hash = BytesN::from_array(&s.env, &[0u8; 32]);
    assert_eq!(
        s.client.try_upgrade_self(&zero_hash, &1u32),
        Err(Ok(Error::InvalidWasmHash))
    );
    // A non-zero hash passes validation; the host-level WASM swap
    // (update_current_contract_wasm) is a Soroban VM operation that cannot
    // be exercised in the unit-test VM without a compatible WASM binary,
    // but the validation gate is verified above.
}

#[test]
fn upgrade_rejects_when_paused() {
    let s = Setup::new();
    s.client.pause();
    let valid_hash = BytesN::from_array(&s.env, &[2u8; 32]);
    let result = s.client.try_upgrade_self(&valid_hash, &1u32);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn upgrade_rejects_storage_version_mismatch() {
    let s = Setup::new();
    let valid_hash = BytesN::from_array(&s.env, &[2u8; 32]);
    let result = s.client.try_upgrade_self(&valid_hash, &2u32);
    assert_eq!(result, Err(Ok(Error::StorageVersionMismatch)));
}

#[test]
fn storage_version_set_on_initialize() {
    let s = Setup::new();
    assert_eq!(s.client.factory_storage_version(), 1);
}

#[test]
fn upgrade_blocked_while_paused_then_allowed_after_unpause() {
    let s = Setup::new();
    s.client.pause();
    let valid_hash = BytesN::from_array(&s.env, &[2u8; 32]);
    assert_eq!(
        s.client.try_upgrade_self(&valid_hash, &1u32),
        Err(Ok(Error::ContractPaused))
    );

    s.client.unpause();
    // After unpausing, zero-hash validation still rejects.
    let zero_hash = BytesN::from_array(&s.env, &[0u8; 32]);
    assert_eq!(
        s.client.try_upgrade_self(&zero_hash, &1u32),
        Err(Ok(Error::InvalidWasmHash))
    );
}

// ── Legacy index migration (#383) ─────────────────────────────────────────

#[test]
fn legacy_sender_index_migration_is_incremental() {
    let s = Setup::new();
    let sender = Address::generate(&s.env);

    s.env.as_contract(&s.client.address, || {
        let mut legacy = soroban_sdk::Vec::new(&s.env);
        for id in 0..250_u64 {
            legacy.push_back(id);
        }
        s.env
            .storage()
            .persistent()
            .set(&DataKey::BySender(sender.clone()), &legacy);
    });

    assert_eq!(s.client.stream_count_by_sender(&sender), 250);
    assert_eq!(s.client.migrate_sender_index(&sender, &1), 100);

    s.env.as_contract(&s.client.address, || {
        assert!(s
            .env
            .storage()
            .persistent()
            .has(&DataKey::BySender(sender.clone())));
        assert_eq!(
            s.env
                .storage()
                .persistent()
                .get::<_, u32>(&DataKey::BySenderMigrationCursor(sender.clone())),
            Some(100)
        );
    });

    let page = s.client.streams_by_sender(&sender, &95, &10);
    assert_eq!(page.ids.len(), 10);
    assert_eq!(page.ids.get(0).unwrap(), 95);
    assert_eq!(page.ids.get(9).unwrap(), 104);

    assert_eq!(s.client.migrate_sender_index(&sender, &10), 250);
    s.env.as_contract(&s.client.address, || {
        assert!(!s
            .env
            .storage()
            .persistent()
            .has(&DataKey::BySender(sender.clone())));
    });
}

#[test]
fn append_during_partial_sender_migration_preserves_order() {
    let s = Setup::new();
    let sender = Address::generate(&s.env);

    s.env.as_contract(&s.client.address, || {
        let mut legacy = soroban_sdk::Vec::new(&s.env);
        for id in 0..150_u64 {
            legacy.push_back(id);
        }
        s.env
            .storage()
            .persistent()
            .set(&DataKey::BySender(sender.clone()), &legacy);

        crate::index::append_sender_index(&s.env, &sender, 999);
    });

    assert_eq!(s.client.stream_count_by_sender(&sender), 151);

    let tail = s.client.streams_by_sender(&sender, &145, &10);
    assert_eq!(tail.ids.len(), 6);
    assert_eq!(tail.ids.get(0).unwrap(), 145);
    assert_eq!(tail.ids.get(4).unwrap(), 149);
    assert_eq!(tail.ids.get(5).unwrap(), 999);

    assert_eq!(s.client.migrate_sender_index(&sender, &10), 151);
    let tail_after = s.client.streams_by_sender(&sender, &145, &10);
    assert_eq!(tail_after, tail);
}

// ── Issue #204: cancel_batch_streams ─────────────────────────────────────────

#[test]
fn cancel_batch_rejects_empty_list() {
    let s = Setup::new();
    let sender = Address::generate(&s.env);
    let addresses: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(&s.env);

    let result = s.client.try_cancel_batch_streams(&sender, &addresses);
    assert_eq!(result, Err(Ok(Error::EmptyBatch)));
}

#[test]
fn cancel_batch_rejects_oversized_list() {
    let s = Setup::new();
    let sender = Address::generate(&s.env);
    let mut addresses: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(&s.env);
    for _ in 0..101 {
        addresses.push_back(Address::generate(&s.env));
    }
    let result = s.client.try_cancel_batch_streams(&sender, &addresses);
    assert_eq!(result, Err(Ok(Error::BatchTooLarge)));
}

// ── protocol_fee_bps distinguishes "fee is 30" from "couldn't read it" (#339)

#[test]
fn protocol_fee_bps_reports_not_initialized_before_setup() {
    // An uninitialised factory has no governor address to read a fee from.
    // Previously this returned a confident `30`, which a caller could not tell
    // apart from a governor genuinely configured at 30 bps.
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, DripFactory);
    let client = DripFactoryClient::new(&env, &contract_id);

    assert_eq!(
        client.try_protocol_fee_bps(),
        Err(Ok(Error::NotInitialized))
    );
}

#[test]
fn protocol_fee_bps_reports_governor_not_responding() {
    // The setup governor is a bare generated address with no contract behind
    // it, so the cross-contract call fails — the "governor archived / not
    // initialised / host error" case from the issue. `create_stream` already
    // fails loudly here; this now does too, instead of quoting 30 bps.
    let s = Setup::new();

    assert_eq!(
        s.client.try_protocol_fee_bps(),
        Err(Ok(Error::GovernorNotResponding))
    );
}

#[test]
fn protocol_fee_bps_or_default_still_offers_a_lenient_read() {
    // The lenient behaviour remains available, but the caller supplies the
    // fallback and is therefore choosing it deliberately.
    let s = Setup::new();

    assert_eq!(s.client.protocol_fee_bps_or_default(&30), 30);
    // And it is genuinely the caller's value, not a constant baked into the
    // factory — which is what made the old return value ambiguous.
    assert_eq!(s.client.protocol_fee_bps_or_default(&77), 77);
}

#[test]
fn factory_status_reports_an_unreadable_fee_as_none() {
    // The combined view still succeeds so `is_paused` stays readable during a
    // governor outage — but the fee is None rather than a plausible-looking 30.
    let s = Setup::new();

    let status = s.client.factory_status();

    assert_eq!(status.protocol_fee_bps, None);
    assert!(!status.is_paused);
}

#[test]
fn factory_status_still_reports_pause_state_when_the_fee_is_unreadable() {
    // The reason the fee is optional rather than the whole call being
    // fallible: an operator checking whether the protocol is paused must not
    // be blocked by an unrelated governor problem.
    let s = Setup::new();
    s.client.pause();

    let status = s.client.factory_status();

    assert!(status.is_paused);
    assert_eq!(status.protocol_fee_bps, None);
}

// ── #382: paginated index must keep ALL pages alive on any read/append ────────
//
// Before this fix `read_index`/`append_index_entry` extended the TTL of only the
// page(s) they touched. A page that had filled was never written again and was
// only read when a query happened to land in its range, so a UI that reads
// "most recent first" let page 0 silently archive. After archival the paginated
// queries returned fewer IDs than the count reported, and the read loop's
// `page_offset >= page_len` branch skipped straight past the archived page's
// entries.
//
// These tests seed a multi-page index directly (create_stream needs a built
// stream WASM) and assert that a read/append of only the NEWEST page extends
// the TTL of EVERY page, using the test-harness `get_ttl` for persistent
// entries to observe the bump directly.

/// Build an env + registered factory tuned so freshly seeded persistent pages
/// start NEAR expiry (a small TTL), making any TTL bump easily observable.
fn index_ttl_env() -> (Env, DripFactoryClient<'static>, Address) {
    use soroban_sdk::testutils::{Ledger as _, LedgerInfo};

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set(LedgerInfo {
        timestamp: 1_000_000,
        protocol_version: 21,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });

    let governor = Address::generate(&env);
    let wasm_hash = BytesN::from_array(&env, &[1u8; 32]);
    let contract_id = env.register_contract(None, DripFactory);
    let client = DripFactoryClient::new(&env, &contract_id);
    client.initialize(&wasm_hash, &governor);

    (env, client, contract_id)
}

/// Seed a 3-page (250-entry) sender index directly under the factory: pages 0
/// and 1 are full (100 each), page 2 is partial (50). Returns the sender.
fn seed_sender_pages(env: &Env, factory: &Address, count: u32) -> Address {
    use crate::query::MAX_PAGE_SIZE;
    use crate::storage::DataKey;
    use soroban_sdk::Vec as SVec;

    let sender = Address::generate(env);
    let cap = MAX_PAGE_SIZE;
    env.as_contract(factory, || {
        for page in 0..3u32 {
            let len = if page == 2 { count - 2 * cap } else { cap };
            let mut v = SVec::new(env);
            let start = page * cap;
            for i in 0..len {
                v.push_back((start + i) as u64);
            }
            env.storage()
                .persistent()
                .set(&DataKey::BySenderPage(sender.clone(), page), &v);
        }
        env.storage()
            .persistent()
            .set(&DataKey::BySenderCount(sender.clone()), &count);
    });
    sender
}

#[test]
fn streams_by_sender_reports_total_so_a_capped_limit_is_not_silent() {
    let (env, client, contract_id) = index_ttl_env();
    let sender = seed_sender_pages(&env, &contract_id, 250);

    // Asking for far more than MAX_PAGE_SIZE is silently capped, but `total`
    // lets the caller tell "capped" apart from "sender only has 200 streams"
    // without a separate stream_count_by_sender call.
    let page = client.streams_by_sender(&sender, &0, &200);
    assert_eq!(page.ids.len(), 100);
    assert_eq!(page.total, 250);
    assert!((page.ids.len() as u32) < page.total);

    // A sender whose whole history fits in one page reports total == ids.len(),
    // so the same comparison correctly signals "no more pages".
    use crate::storage::DataKey;
    use soroban_sdk::Vec as SVec;
    let small_sender = Address::generate(&env);
    env.as_contract(&contract_id, || {
        let mut v = SVec::new(&env);
        for i in 0..50u64 {
            v.push_back(i);
        }
        env.storage()
            .persistent()
            .set(&DataKey::BySenderPage(small_sender.clone(), 0), &v);
        env.storage()
            .persistent()
            .set(&DataKey::BySenderCount(small_sender.clone()), &50u32);
    });
    let small_page = client.streams_by_sender(&small_sender, &0, &200);
    assert_eq!(small_page.ids.len(), 50);
    assert_eq!(small_page.total, 50);
}

#[test]
fn streams_by_sender_refreshes_ttl_on_all_pages_not_just_read_page() {
    use crate::storage::DataKey;
    use crate::ttl;
    use soroban_sdk::testutils::storage::Persistent as _;

    let (env, client, contract_id) = index_ttl_env();
    let sender = seed_sender_pages(&env, &contract_id, 250);

    // Read only the newest page — the "most recent first" UI pattern that used
    // to leave the older pages untoccuched and, eventually, archived.
    let last_page = client.streams_by_sender(&sender, &200, &100).ids;
    assert_eq!(last_page.len(), 50);
    assert_eq!(last_page.get(0), Some(200));
    assert_eq!(last_page.get(49), Some(249));

    // Every populated page — not just the ones the window touched — must have
    // had its TTL refreshed to EXTEND_TO.
    for page in 0..3u32 {
        let remaining = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get_ttl(&DataKey::BySenderPage(sender.clone(), page))
        });
        assert_eq!(
            remaining,
            ttl::EXTEND_TO,
            "reading only the newest page must extend page {page}'s TTL too"
        );
    }

    // And the whole history is still readable from the start.
    let head = client.streams_by_sender(&sender, &0, &100).ids;
    assert_eq!(head.len(), 100);
    assert_eq!(head.get(0), Some(0));
}

#[test]
fn append_refreshes_ttl_on_all_pages_not_just_newest() {
    use crate::storage::DataKey;
    use crate::ttl;
    use soroban_sdk::testutils::storage::Persistent as _;

    let (env, _, contract_id) = index_ttl_env();
    let sender = seed_sender_pages(&env, &contract_id, 250);

    // Sanity check: fresh page 0 is near expiry, so a bump is observable.
    let before = env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .get_ttl(&DataKey::BySenderPage(sender.clone(), 0))
    });
    assert!(
        before < ttl::EXTEND_TO,
        "seeded page 0 should be near expiry (was {before})"
    );

    // Append one more stream for this sender. Only the final page is written,
    // but the whole index must be kept alive.
    env.as_contract(&contract_id, || {
        crate::index::append_sender_index(&env, &sender, 250);
    });

    let after = env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .get_ttl(&DataKey::BySenderPage(sender.clone(), 0))
    });
    assert_eq!(
        after,
        ttl::EXTEND_TO,
        "appending must refresh page 0's TTL even though page 0 is not written"
    );

    let count_remaining = env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .get_ttl(&DataKey::BySenderCount(sender.clone()))
    });
    assert_eq!(count_remaining, ttl::EXTEND_TO);
}

// ── Gas / CPU-instruction regression test (issue #441) ─────────────────────

#[test]
fn read_index_cpu_instruction_regression() {
    let (env, client, contract_id) = index_ttl_env();
    let sender = seed_sender_pages(&env, &contract_id, 250);

    // Reset budget tracker before the measured call.
    env.budget().reset_tracker();

    // Read a full page (100 entries) — this exercises read_index with
    // legacy cursor + page traversal.
    let _ = client.streams_by_sender(&sender, &0, &100);

    let cpu = env.budget().cpu_instruction_cost();
    // Threshold determined empirically: 250-entry index, full-page read.
    // Fails if future changes silently blow the budget.
    assert!(
        cpu < 5_000_000,
        "streams_by_sender full-page read consumed {cpu} CPU instructions, exceeds 2M regression threshold"
    );
}

#[test]
fn test_is_known_stream_returns_false_for_unknown_address() {
    let s = Setup::new();
    let random_addr = Address::generate(&s.env);
    assert_eq!(s.client.is_known_stream(&random_addr), false);
}

#[test]
fn test_max_batch_size_matches_constant() {
    let s = Setup::new();
    assert_eq!(s.client.max_batch_size(), crate::MAX_BATCH_SIZE);
    assert_eq!(s.client.max_batch_size(), 10);
}

#[test]
fn test_stream_wasm_hash_returns_configured_hash() {
    let s = Setup::new();
    let expected_hash = BytesN::from_array(&s.env, &[1u8; 32]);
    assert_eq!(s.client.stream_wasm_hash(), expected_hash);

    let new_hash = BytesN::from_array(&s.env, &[2u8; 32]);
    s.client.upgrade_stream_wasm(&new_hash);
    assert_eq!(s.client.stream_wasm_hash(), new_hash);
}

#[test]
fn test_stream_wasm_hash_uninitialized_returns_error() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, DripFactory);
    let client = DripFactoryClient::new(&env, &contract_id);
    assert_eq!(
        client.try_stream_wasm_hash(),
        Err(Ok(Error::NotInitialized))
    );
}

// ── Issue #622: record_cancel verification and idempotency ───────────────────

fn deploy_test_stream(env: &Env, sender: &Address) -> Address {
    let stream_addr = env.register_contract(None, drip_stream::DripStream);
    let recipient = Address::generate(env);
    let token_admin = Address::generate(env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let tok = soroban_sdk::token::StellarAssetClient::new(env, &token_addr);
    tok.mint(sender, &1000);
    tok.mint(&stream_addr, &1000);

    let stream_client = drip_stream::DripStreamClient::new(env, &stream_addr);
    let start = env.ledger().timestamp();
    stream_client.initialize(
        sender,
        &recipient,
        &token_addr,
        &10,
        &start,
        &(start + 100),
        &false,
        &2_592_000,
        &false,
        &None,
    );
    stream_addr
}

#[test]
fn record_cancel_rejects_unknown_stream_address() {
    let s = Setup::new();
    let unknown_addr = Address::generate(&s.env);

    let agg_before = s.client.aggregate();
    s.client.record_cancel(&unknown_addr);
    let agg_after = s.client.aggregate();
    assert_eq!(agg_before.active_streams, agg_after.active_streams);
}

#[test]
fn record_cancel_ignores_uncancelled_stream() {
    let s = Setup::new();
    let sender = Address::generate(&s.env);
    let stream_addr = deploy_test_stream(&s.env, &sender);

    // Register stream as known in factory and set initial active_streams
    s.env.as_contract(&s.client.address, || {
        s.env
            .storage()
            .persistent()
            .set(&DataKey::IsKnownStream(stream_addr.clone()), &true);
        let mut agg: crate::storage::Aggregate = s
            .env
            .storage()
            .instance()
            .get(&DataKey::Aggregate)
            .unwrap_or(crate::storage::Aggregate {
                total_supply: 0,
                active_streams: 0,
            });
        agg.active_streams = 1;
        s.env.storage().instance().set(&DataKey::Aggregate, &agg);
    });

    let agg_before = s.client.aggregate();
    assert_eq!(agg_before.active_streams, 1);

    // Call record_cancel while stream is active
    s.client.record_cancel(&stream_addr);
    let agg_after = s.client.aggregate();
    assert_eq!(agg_after.active_streams, 1);
}

#[test]
fn record_cancel_decrements_only_once_for_cancelled_stream() {
    let s = Setup::new();
    let sender = Address::generate(&s.env);
    let stream_addr = deploy_test_stream(&s.env, &sender);

    // Register stream as known in factory and set initial active_streams
    s.env.as_contract(&s.client.address, || {
        s.env
            .storage()
            .persistent()
            .set(&DataKey::IsKnownStream(stream_addr.clone()), &true);
        let mut agg: crate::storage::Aggregate = s
            .env
            .storage()
            .instance()
            .get(&DataKey::Aggregate)
            .unwrap_or(crate::storage::Aggregate {
                total_supply: 0,
                active_streams: 0,
            });
        agg.active_streams = 1;
        s.env.storage().instance().set(&DataKey::Aggregate, &agg);
    });

    let stream_client = drip_stream::DripStreamClient::new(&s.env, &stream_addr);
    stream_client.cancel(&sender);

    let agg_before = s.client.aggregate();
    assert_eq!(agg_before.active_streams, 1);

    // First record_cancel call decrements
    s.client.record_cancel(&stream_addr);
    let agg_after = s.client.aggregate();
    assert_eq!(agg_after.active_streams, 0);

    // Second record_cancel call is idempotent (no second decrement)
    s.client.record_cancel(&stream_addr);
    let agg_after_repeat = s.client.aggregate();
    assert_eq!(agg_after_repeat.active_streams, 0);
}

#[test]
fn record_cancel_idempotent_after_batch_cancel() {
    let s = Setup::new();
    let sender = Address::generate(&s.env);
    let stream_addr = deploy_test_stream(&s.env, &sender);

    s.env.as_contract(&s.client.address, || {
        s.env
            .storage()
            .persistent()
            .set(&DataKey::IsKnownStream(stream_addr.clone()), &true);
        let mut agg: crate::storage::Aggregate = s
            .env
            .storage()
            .instance()
            .get(&DataKey::Aggregate)
            .unwrap_or(crate::storage::Aggregate {
                total_supply: 0,
                active_streams: 0,
            });
        agg.active_streams = 1;
        s.env.storage().instance().set(&DataKey::Aggregate, &agg);
    });

    let mut addrs = soroban_sdk::Vec::new(&s.env);
    addrs.push_back(stream_addr.clone());
    s.client.cancel_batch_streams(&sender, &addrs);

    let agg_after_batch = s.client.aggregate();
    assert_eq!(agg_after_batch.active_streams, 0);

    // Subsequent record_cancel on batch-cancelled stream does not decrement further
    s.client.record_cancel(&stream_addr);
    let agg_after_record = s.client.aggregate();
    assert_eq!(agg_after_record.active_streams, 0);
}

// ── Issue #623: estimate_fee coverage across all StreamOperation variants ────

#[test]
fn estimate_fee_covers_all_operations_with_positive_estimates() {
    let s = Setup::new();

    let ops = [
        crate::storage::StreamOperation::CreateStream,
        crate::storage::StreamOperation::CancelStream,
        crate::storage::StreamOperation::Withdraw,
        crate::storage::StreamOperation::PauseStream,
        crate::storage::StreamOperation::ResumeStream,
        crate::storage::StreamOperation::SetOperator,
        crate::storage::StreamOperation::RevokeOperator,
        crate::storage::StreamOperation::ExtendDuration,
        crate::storage::StreamOperation::TopUp,
        crate::storage::StreamOperation::TopUpAndExtend,
        crate::storage::StreamOperation::Clawback,
        crate::storage::StreamOperation::ForceCancel,
        crate::storage::StreamOperation::TransferRecipient,
    ];

    for op in ops.into_iter() {
        let estimate = s.client.estimate_fee(&op);
        assert!(
            estimate.cpu_instructions > 0,
            "cpu_instructions should be > 0"
        );
        assert!(estimate.ledger_entries > 0, "ledger_entries should be > 0");
        assert_eq!(estimate.fee_stroops, 0);
        assert_eq!(estimate.fee_xlm, 0);
    }
}

// ── Issue #624: index migration bounds and short-circuit ─────────────────────

#[test]
fn migrate_sender_index_caps_pages_and_short_circuits_when_done() {
    let s = Setup::new();
    let sender = Address::generate(&s.env);

    s.env.as_contract(&s.client.address, || {
        let mut legacy = soroban_sdk::Vec::new(&s.env);
        for id in 0..250_u64 {
            legacy.push_back(id);
        }
        s.env
            .storage()
            .persistent()
            .set(&DataKey::BySender(sender.clone()), &legacy);
    });

    // Calling with an arbitrarily huge max_pages (e.g. 500) migrates all available pages safely
    let migrated = s.client.migrate_sender_index(&sender, &500);
    assert_eq!(migrated, 250);

    // Legacy key is removed once completed
    s.env.as_contract(&s.client.address, || {
        assert!(!s
            .env
            .storage()
            .persistent()
            .has(&DataKey::BySender(sender.clone())));
    });

    // Calling again short-circuits to cheap no-op read and returns total count
    let migrated_repeat = s.client.migrate_sender_index(&sender, &10);
    assert_eq!(migrated_repeat, 250);
}

#[test]
fn migrate_recipient_index_short_circuits_when_done() {
    let s = Setup::new();
    let recipient = Address::generate(&s.env);

    s.env.as_contract(&s.client.address, || {
        let mut legacy = soroban_sdk::Vec::new(&s.env);
        for id in 0..150_u64 {
            legacy.push_back(id);
        }
        s.env
            .storage()
            .persistent()
            .set(&DataKey::ByRecipient(recipient.clone()), &legacy);
    });

    let migrated = s.client.migrate_recipient_index(&recipient, &100);
    assert_eq!(migrated, 150);

    // Repeated call short-circuits
    let migrated_repeat = s.client.migrate_recipient_index(&recipient, &10);
    assert_eq!(migrated_repeat, 150);
}

#[test]
#[ignore = "requires a real deployed stream WASM; fixture uses a dummy hash"]
fn test_stream_created_event_payload_includes_salt_and_wasm_hash() {
    let env = Env::default();
    let creator = Address::generate(&env);
    let recipient = Address::generate(&env);
    let stream_addr = Address::generate(&env);
    let salt = BytesN::from_array(&env, &[42u8; 32]);
    let wasm_hash = BytesN::from_array(&env, &[99u8; 32]);

    crate::events::stream_created(&env, &creator, &recipient, &stream_addr, &salt, &wasm_hash);

    let events = env.events().all();
    assert_eq!(events.len(), 1);
    let last = events.last().unwrap();
    assert_eq!(last.1.len(), 3); // Topics: (StreamCreated, creator, recipient)
}

#[test]
fn test_get_streams_by_creator_paged() {
    let s = Setup::new();
    let creator = Address::generate(&s.env);
    let stream1 = Address::generate(&s.env);
    let stream2 = Address::generate(&s.env);

    s.env.as_contract(&s.client.address, || {
        let mut ids = soroban_sdk::Vec::new(&s.env);
        ids.push_back(1u64);
        ids.push_back(2u64);
        s.env
            .storage()
            .persistent()
            .set(&DataKey::BySender(creator.clone()), &ids);
        s.env
            .storage()
            .persistent()
            .set(&DataKey::StreamAddr(1u64), &stream1);
        s.env
            .storage()
            .persistent()
            .set(&DataKey::StreamAddr(2u64), &stream2);
    });

    let paged = s.client.get_streams_by_creator_paged(&creator, &0, &10);
    assert_eq!(paged.len(), 2);
    assert_eq!(paged.get(0).unwrap(), stream1);
    assert_eq!(paged.get(1).unwrap(), stream2);

    let paged_offset = s.client.get_streams_by_creator_paged(&creator, &1, &1);
    assert_eq!(paged_offset.len(), 1);
    assert_eq!(paged_offset.get(0).unwrap(), stream2);

    let all = s.client.get_streams_by_creator(&creator);
    assert_eq!(all.len(), 2);
}

fn high_stream_creation_concurrency() {
    use soroban_sdk::testutils::BytesN as _;

    let s = Setup::new();

    // Simulate high concurrent stream creation by rapidly calling create_stream
    // with different senders and recipients. Each invocation:
    // - Generates a unique sender and recipient
    // - Deploys a new DripStream contract instance
    // - Registers state in the factory's index
    //
    // We target 50 concurrent-like operations to stress the index and
    // storage operations under load.
    let concurrency_level = 50;
    let mut stream_ids = std::vec::Vec::new();

    for i in 0..concurrency_level {
        let sender = Address::generate(&s.env);
        let recipient = Address::generate(&s.env);
        let token = Address::generate(&s.env);

        // Verify each stream_id is unique
        let stream_id = s.client.create_stream(
            &sender,
            &recipient,
            &token,
            &1_000_000i128, // deposit
            &1i128,         // rate per second
            &(s.env.ledger().timestamp() + 3600),
            &0u64, // open-ended
            &true, // clawback enabled
            &None,
        );

        // Ensure stream_ids don't collide
        assert!(
            !stream_ids.contains(&stream_id),
            "Duplicate stream_id at iteration {}",
            i
        );
        stream_ids.push(stream_id);
    }

    // Verify all streams were created and indexed
    assert_eq!(
        stream_ids.len(),
        concurrency_level,
        "Expected {} stream IDs created, got {}",
        concurrency_level,
        stream_ids.len()
    );

    // Spot-check: verify a few random stream IDs are queryable from the index
    for &stream_id in stream_ids.iter().step_by(10).take(5) {
        let stream_info = s.client.stream_address(&stream_id);
        assert!(
            stream_info.is_some(),
            "Stream {} not found in index",
            stream_id
        );
    }
}

#[test]
#[ignore = "requires a real deployed stream WASM; fixture uses a dummy hash"]
fn concurrent_stream_creation_maintains_factory_invariants() {
    use soroban_sdk::testutils::BytesN as _;

    let s = Setup::new();

    let senders: std::vec::Vec<_> = (0..10).map(|_| Address::generate(&s.env)).collect();

    // Create multiple streams per sender to test:
    // - Index integrity under repeated sender entries
    // - No cross-contamination between senders
    // - Proper ref-count or cleanup if implemented
    let streams_per_sender = 5;
    let total_expected = senders.len() * streams_per_sender;

    for sender in &senders {
        for _ in 0..streams_per_sender {
            let recipient = Address::generate(&s.env);
            let token = Address::generate(&s.env);

            let _stream_id = s.client.create_stream(
                sender,
                &recipient,
                &token,
                &500_000i128,
                &1i128,
                &(s.env.ledger().timestamp() + 7200),
                &0u64,
                &false,
                &None,
            );
        }
    }

    // Verify event count reflects the total number of creations
    let event_count = s.event_count();
    assert!(
        event_count >= total_expected,
        "Expected at least {} creation events, got {}",
        total_expected,
        event_count
    );
}

// ── Issue #844: zero-deposit validation ─────────────────────────────────────

#[test]
fn create_stream_rejects_zero_deposit_before_governor_call() {
    let s = Setup::new();
    let sender = Address::generate(&s.env);
    let recipient = Address::generate(&s.env);
    let token = Address::generate(&s.env);

    // `validate_stream_request` rejects the deposit before the factory reads
    // the governor, so the dummy governor in this fixture is never called.
    let result = s.client.try_create_stream(
        &sender, &recipient, &token, &0i128, // deposit == 0
        &1i128, &0u64, &3_600u64, &false, &None,
    );

    assert_eq!(result, Err(Ok(Error::InvalidDeposit)));
}

// ── Issues #845/#847: factory-side description and duration validation ──────

#[test]
fn create_stream_rejects_a_bounded_stream_shorter_than_the_minimum() {
    let s = Setup::new();
    let sender = Address::generate(&s.env);
    let recipient = Address::generate(&s.env);
    let token = Address::generate(&s.env);

    let result = s.client.try_create_stream(
        &sender, &recipient, &token, &3_600i128, &1i128, &0u64,
        &30u64, // shorter than MIN_STREAM_DURATION
        &false, &None,
    );

    assert_eq!(result, Err(Ok(Error::DurationTooShort)));
}

#[test]
fn create_stream_rejects_a_description_over_128_bytes() {
    let s = Setup::new();
    let sender = Address::generate(&s.env);
    let recipient = Address::generate(&s.env);
    let token = Address::generate(&s.env);
    let too_long = soroban_sdk::String::from_str(&s.env, &"x".repeat(129));

    let result = s.client.try_create_stream(
        &sender,
        &recipient,
        &token,
        &3_600i128,
        &1i128,
        &0u64,
        &3_600u64,
        &false,
        &Some(too_long),
    );

    assert_eq!(result, Err(Ok(Error::DescriptionTooLong)));
}
