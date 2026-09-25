//! Per-entry-point instruction-count snapshot.
//!
//! Run with `UPDATE_GAS_SNAPSHOT=1 cargo test -p conduit-integration-tests --test gas_snapshot`
//! to regenerate `.gas-snapshot.json`. CI runs the same test without the
//! update flag and fails if the committed snapshot diverges, making cost
//! regressions visible in review.
//!
//! The measured entry points include `BatchTransferProcessor::process_batch`
//! at `MAX_BATCH_SIZE` (100 transfers) — the largest batch a client can
//! submit in one transaction. The test host's default budget is the same as
//! the on-chain per-transaction limits (100M CPU instructions / 40 MiB), so a
//! batch that would not fit in a single transaction fails this test outright,
//! and the committed baseline pins its exact cost.

use drip_batch_processor::{BatchTransferProcessor, BatchTransferProcessorClient};
use drip_factory::{DripFactory, DripFactoryClient};
use drip_governor::{DripGovernor, DripGovernorClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token, Address, BytesN, Env,
};
use std::collections::BTreeMap;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Snapshot {
    /// Human-readable comment explaining how the snapshot is produced.
    comment: String,
    /// Measured CPU instruction counts per entry point.
    instructions: BTreeMap<String, u64>,
}

fn base_env() -> Env {
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
    env
}

fn deploy_factory(env: &Env) -> DripFactoryClient<'_> {
    let factory_id = env.register_contract(None, DripFactory);
    let governor_id = env.register_contract(None, DripGovernor);

    let authority = Address::generate(env);
    let fee_recipient = Address::generate(env);
    let governor_client = DripGovernorClient::new(env, &governor_id);
    governor_client.initialize(&authority, &fee_recipient, &factory_id);

    let client = DripFactoryClient::new(env, &factory_id);
    let dummy_hash = BytesN::from_array(env, &[1u8; 32]);
    client.initialize(&dummy_hash, &governor_id);
    client
}

/// A batch of `n` distinct recipients each receiving `1` token, sized to the
/// `MAX_BATCH_SIZE` boundary without the std `vec!` macro (the SDK vector
/// needs an explicit `Env`).
fn batch_inputs(env: &Env, n: u32) -> (soroban_sdk::Vec<Address>, soroban_sdk::Vec<i128>) {
    let mut recipients = soroban_sdk::Vec::new(env);
    let mut amounts = soroban_sdk::Vec::new(env);
    for _ in 0..n {
        recipients.push_back(Address::generate(env));
        amounts.push_back(1);
    }
    (recipients, amounts)
}

#[test]
fn gas_snapshot_matches_committed_file() {
    let env = base_env();
    let factory = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);

    let mut instructions = BTreeMap::new();

    // Read-only entry points.
    env.budget().reset_default();
    factory.stream_count();
    instructions.insert("stream_count".into(), env.budget().cpu_instruction_cost());

    env.budget().reset_default();
    factory.protocol_fee_bps();
    instructions.insert(
        "protocol_fee_bps".into(),
        env.budget().cpu_instruction_cost(),
    );

    env.budget().reset_default();
    factory.streams_by_sender(&sender, &0, &10);
    instructions.insert(
        "streams_by_sender".into(),
        env.budget().cpu_instruction_cost(),
    );

    env.budget().reset_default();
    factory.streams_by_recipient(&recip, &0, &10);
    instructions.insert(
        "streams_by_recipient".into(),
        env.budget().cpu_instruction_cost(),
    );

    env.budget().reset_default();
    factory.aggregate();
    instructions.insert("aggregate".into(), env.budget().cpu_instruction_cost());

    // State-mutating entry points.
    env.budget().reset_default();
    factory.pause();
    instructions.insert("pause".into(), env.budget().cpu_instruction_cost());

    env.budget().reset_default();
    factory.unpause();
    instructions.insert("unpause".into(), env.budget().cpu_instruction_cost());

    // ── BatchTransferProcessor (#563) ───────────────────────────────────────
    let processor_id = env.register_contract(None, BatchTransferProcessor);
    let processor = BatchTransferProcessorClient::new(&env, &processor_id);

    env.budget().reset_default();
    processor.version();
    instructions.insert(
        "batch_processor/version".into(),
        env.budget().cpu_instruction_cost(),
    );

    env.budget().reset_default();
    processor.max_batch_size();
    instructions.insert(
        "batch_processor/max_batch_size".into(),
        env.budget().cpu_instruction_cost(),
    );

    // Worst case a client can submit: a batch at `MAX_BATCH_SIZE`. Each
    // entry is a separate SEP-41 transfer after the single inbound pull, so
    // this is the most expensive call the processor exposes.
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let batch_funder = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token_addr).mint(&batch_funder, &100);
    let (recipients, amounts) = batch_inputs(&env, 100);

    env.budget().reset_default();
    processor.process_batch(&batch_funder, &token_addr, &recipients, &amounts);
    instructions.insert(
        "batch_processor/process_batch_100".into(),
        env.budget().cpu_instruction_cost(),
    );

    let snapshot_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".gas-snapshot.json");
    let update = std::env::var("UPDATE_GAS_SNAPSHOT").is_ok();

    if update {
        let snapshot = Snapshot {
            comment: "Auto-generated by tests/gas_snapshot.rs. Do not hand-edit. \
                      Run with UPDATE_GAS_SNAPSHOT=1 to regenerate."
                .into(),
            instructions,
        };
        std::fs::write(
            &snapshot_path,
            serde_json::to_string_pretty(&snapshot).unwrap(),
        )
        .expect("write snapshot");
    } else {
        let committed: Snapshot = serde_json::from_str(
            &std::fs::read_to_string(&snapshot_path)
                .expect("missing .gas-snapshot.json; run with UPDATE_GAS_SNAPSHOT=1"),
        )
        .expect("invalid snapshot JSON");

        // A newly tracked entry point with no committed baseline would
        // otherwise pass unnoticed — the comparison below only looks at keys
        // present in both files.
        let missing: std::vec::Vec<&str> = instructions
            .keys()
            .filter(|k| !committed.instructions.contains_key(*k))
            .map(String::as_str)
            .collect();
        assert!(
            missing.is_empty(),
            "gas snapshot has no committed baseline for: {:?} \
             (run with UPDATE_GAS_SNAPSHOT=1)",
            missing,
        );

        let stale: std::vec::Vec<&str> = committed
            .instructions
            .keys()
            .filter(|k| !instructions.contains_key(*k))
            .map(String::as_str)
            .collect();
        assert!(
            stale.is_empty(),
            "gas snapshot has entries no longer measured: {:?} \
             (run with UPDATE_GAS_SNAPSHOT=1)",
            stale,
        );

        let diff: BTreeMap<String, (u64, u64)> = instructions
            .iter()
            .filter_map(|(k, v)| {
                committed.instructions.get(k).and_then(|committed| {
                    if committed != v {
                        Some((k.clone(), (*committed, *v)))
                    } else {
                        None
                    }
                })
            })
            .collect();

        assert!(
            diff.is_empty(),
            "gas snapshot diff detected (committed -> current): {:?}",
            diff
        );
    }
}
