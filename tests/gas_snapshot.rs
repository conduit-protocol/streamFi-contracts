//! Per-entry-point instruction-count snapshot.
//!
//! Run with `UPDATE_GAS_SNAPSHOT=1 cargo test -p conduit-integration-tests --test gas_snapshot`
//! to regenerate `.gas-snapshot.json`. CI runs the same test without the
//! update flag and fails if the committed snapshot diverges, making cost
//! regressions visible in review.
//!
//! Covers `factory` (see `deploy_factory`), `oracle` (`oracle_*` keys,
//! `deploy_oracle`), `token-vault` (`vault_*` keys, `deploy_token_vault`), and
//! `batch-processor` (`batch_*` keys, `deploy_batch_processor`).

use drip_batch_processor::{BatchTransferProcessor, BatchTransferProcessorClient};
use drip_factory::{DripFactory, DripFactoryClient};
use drip_governor::{DripGovernor, DripGovernorClient};
use drip_oracle::{OracleConfig, Role as OracleRole, TwapOracle, TwapOracleClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token, Address, BytesN, Env, Vec,
};
use std::collections::BTreeMap;
use token_vault::{TokenVault, TokenVaultClient};

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

fn deploy_oracle(env: &Env) -> (TwapOracleClient<'_>, Address) {
    let oracle_id = env.register_contract(None, TwapOracle);
    let client = TwapOracleClient::new(env, &oracle_id);

    let admin = Address::generate(env);
    client.initialize(&admin);
    client.grant_role(&admin, &OracleRole::PriceFeeder, &admin);
    client.configure_oracle(
        &admin,
        &OracleConfig {
            decimals: 8,
            asset_peg: 1,
            max_staleness: 300,
            max_price: 0,
            min_submit_interval: 0,
        },
    );
    (client, admin)
}

fn deploy_token_vault(env: &Env) -> (TokenVaultClient<'_>, Address) {
    let owner = Address::generate(env);
    let token_admin = Address::generate(env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    token::StellarAssetClient::new(env, &token_addr).mint(&owner, &1_000_000_000);

    let max_limit: i128 = 1_000_000_000;
    let vault_id = env.register_contract(None, TokenVault);
    let client = TokenVaultClient::new(env, &vault_id);
    client.initialize(&owner, &token_addr, &max_limit);
    (client, owner)
}

fn deploy_batch_processor(env: &Env) -> (BatchTransferProcessorClient<'_>, Address, Address) {
    let funder = Address::generate(env);
    let token_admin = Address::generate(env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    token::StellarAssetClient::new(env, &token_addr).mint(&funder, &1_000_000_000);

    let contract_id = env.register_contract(None, BatchTransferProcessor);
    let client = BatchTransferProcessorClient::new(env, &contract_id);
    (client, funder, token_addr)
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

    // ── TwapOracle ("oracle_*") ────────────────────────────────────────────
    let (oracle, oracle_admin) = deploy_oracle(&env);

    env.budget().reset_default();
    oracle.submit_price(&oracle_admin, &100_000_000);
    instructions.insert(
        "oracle_submit_price".into(),
        env.budget().cpu_instruction_cost(),
    );

    env.budget().reset_default();
    oracle.get_twap_price();
    instructions.insert(
        "oracle_get_twap_price".into(),
        env.budget().cpu_instruction_cost(),
    );

    env.budget().reset_default();
    oracle.price_status();
    instructions.insert(
        "oracle_price_status".into(),
        env.budget().cpu_instruction_cost(),
    );

    env.budget().reset_default();
    oracle.pause(&oracle_admin);
    instructions.insert("oracle_pause".into(), env.budget().cpu_instruction_cost());

    env.budget().reset_default();
    oracle.unpause(&oracle_admin);
    instructions.insert("oracle_unpause".into(), env.budget().cpu_instruction_cost());

    // ── TokenVault ("vault_*") ──────────────────────────────────────────────
    let (vault, vault_owner) = deploy_token_vault(&env);

    env.budget().reset_default();
    vault.deposit(&vault_owner, &1_000);
    instructions.insert("vault_deposit".into(), env.budget().cpu_instruction_cost());

    env.budget().reset_default();
    vault.owner();
    instructions.insert("vault_owner".into(), env.budget().cpu_instruction_cost());

    env.budget().reset_default();
    vault.withdraw(&vault_owner, &vault_owner, &500);
    instructions.insert("vault_withdraw".into(), env.budget().cpu_instruction_cost());

    env.budget().reset_default();
    vault.pause(&vault_owner);
    instructions.insert("vault_pause".into(), env.budget().cpu_instruction_cost());

    env.budget().reset_default();
    vault.unpause(&vault_owner);
    instructions.insert("vault_unpause".into(), env.budget().cpu_instruction_cost());

    // ── BatchTransferProcessor ("batch_*") ──────────────────────────────────
    let (batch, funder, batch_token_addr) = deploy_batch_processor(&env);

    env.budget().reset_default();
    batch.max_batch_size();
    instructions.insert(
        "batch_max_batch_size".into(),
        env.budget().cpu_instruction_cost(),
    );

    let recipients = Vec::from_array(
        &env,
        [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ],
    );
    let amounts = Vec::from_array(&env, [100i128, 200i128, 300i128]);

    env.budget().reset_default();
    batch.process_batch(&funder, &batch_token_addr, &recipients, &amounts);
    instructions.insert(
        "batch_process_batch".into(),
        env.budget().cpu_instruction_cost(),
    );

    let snapshot_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".gas-snapshot.json");
    let update = std::env::var("UPDATE_GAS_SNAPSHOT").is_ok();

    if update {
        let snapshot = Snapshot {
            comment: "Auto-generated by tests/gas_snapshot.rs. Do not hand-edit.".into(),
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
