#![cfg(test)]

use super::*;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Env, IntoVal,
};

fn create_test_env() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let authority = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let factory_address = Address::generate(&env);

    (env, authority, fee_recipient, factory_address)
}

#[test]
fn test_set_fee_bps_accepts_valid_values() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Test valid fee values
    assert!(client.try_set_fee_bps(&authority, &0).is_ok());
    assert!(client.try_set_fee_bps(&authority, &100).is_ok());
    assert!(client.try_set_fee_bps(&authority, &5000).is_ok());
    assert!(client.try_set_fee_bps(&authority, &10_000).is_ok()); // Exactly 100%
}

#[test]
fn test_set_fee_bps_rejects_values_over_10000() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Test that values over 10,000 (100%) are rejected
    let result = client.try_set_fee_bps(&authority, &10_001);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));

    let result = client.try_set_fee_bps(&authority, &15_000);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));

    let result = client.try_set_fee_bps(&authority, &100_000);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));

    let result = client.try_set_fee_bps(&authority, &u32::MAX);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
fn test_set_fee_bps_boundary_values() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Boundary test: 10,000 should succeed (exactly 100%)
    assert!(client.try_set_fee_bps(&authority, &10_000).is_ok());

    // Boundary test: 10,001 should fail (over 100%)
    let result = client.try_set_fee_bps(&authority, &10_001);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
fn test_set_fee_bps_emits_event() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Set fee and verify event is emitted
    client.set_fee_bps(&authority, &500);

    let events = env.events().all();
    let (_, topics, _data) = events.last().unwrap();

    assert_eq!(
        topics,
        (symbol_short!("fee_bps"), authority.clone()).into_val(&env)
    );
}

#[test]
fn test_set_fee_bps_requires_fee_manager_role() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Create unauthorized user
    let unauthorized = Address::generate(&env);

    // Should fail without FeeManager role
    let result = client.try_set_fee_bps(&unauthorized, &100);
    assert!(result.is_err());
}

#[test]
fn test_set_fee_bps_blocked_when_paused() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Pause the governor
    client.governor_pause(&authority);

    // Should fail when paused
    let result = client.try_set_fee_bps(&authority, &100);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn test_cast_vote_with_rationale_emits_detailed_event() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Create a proposal
    let proposal_id = client.propose(&authority, &3600);

    // Voter casts vote with rationale
    let voter = Address::generate(&env);
    let rationale = soroban_sdk::String::from_str(&env, "ipfs://QmTransparentGovVoteReason123");
    
    assert!(client.try_cast_vote_with_rationale(&voter, &proposal_id, &true, &Some(rationale.clone())).is_ok());

    // Also test cast_vote without rationale
    let voter2 = Address::generate(&env);
    assert!(client.try_cast_vote(&voter2, &proposal_id, &false).is_ok());
}

#[test]
fn test_execute_proposal_enforces_timelock_delay() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    let proposal_id = client.propose(&authority, &3600);

    // Try executing immediately — should fail with TimelockNotMet
    let res = client.try_execute_proposal(&proposal_id);
    assert_eq!(res, Err(Ok(Error::TimelockNotMet)));

    // Advance ledger timestamp to satisfy timelock delay (604_800s)
    let current_time = env.ledger().timestamp();
    env.ledger().set_timestamp(current_time + 604_800);

    // Now execute succeeds
    assert!(client.try_execute_proposal(&proposal_id).is_ok());
}

#[test]
fn test_set_min_quorum_bps_validates_bounds() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Initial default is 0
    assert_eq!(client.min_quorum_bps(), 0);

    // Valid update
    assert!(client.try_set_min_quorum_bps(&authority, &2000).is_ok());
    assert_eq!(client.min_quorum_bps(), 2000);

    // 10,000 (100%) is accepted
    assert!(client.try_set_min_quorum_bps(&authority, &10_000).is_ok());
    assert_eq!(client.min_quorum_bps(), 10_000);

    // Over 10,000 is rejected
    assert_eq!(
        client.try_set_min_quorum_bps(&authority, &10_001),
        Err(Ok(Error::InvalidParam))
    );

    // Non-admin rejected
    let unauthorized = Address::generate(&env);
    assert!(client.try_set_min_quorum_bps(&unauthorized, &500).is_err());
}

#[test]
fn test_execute_proposal_enforces_quorum_threshold() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Set governance supply to 10 and min_quorum_bps to 2000 (20% -> 2 votes needed)
    client.set_governance_supply(&authority, &10);
    client.set_min_quorum_bps(&authority, &2000);

    let proposal_id = client.propose(&authority, &3600);

    // Advance ledger timestamp to satisfy timelock delay
    let current_time = env.ledger().timestamp();
    env.ledger().set_timestamp(current_time + 604_800);

    // 0 votes cast -> fails with QuorumNotMet
    let res = client.try_execute_proposal(&proposal_id);
    assert_eq!(res, Err(Ok(Error::QuorumNotMet)));

    // Cast 1 vote -> total_votes = 1 < 2 -> fails with QuorumNotMet
    let voter1 = Address::generate(&env);
    assert!(client.try_cast_vote(&voter1, &proposal_id, &true).is_ok());
    let res2 = client.try_execute_proposal(&proposal_id);
    assert_eq!(res2, Err(Ok(Error::QuorumNotMet)));

    // Cast 2nd vote (against) -> total_votes = 2 >= 2 -> quorum met!
    let voter2 = Address::generate(&env);
    assert!(client.try_cast_vote(&voter2, &proposal_id, &false).is_ok());

    let proposal = client.get_proposal(&proposal_id);
    assert_eq!(proposal.for_votes, 1);
    assert_eq!(proposal.against_votes, 1);

    // Now execute succeeds!
    assert!(client.try_execute_proposal(&proposal_id).is_ok());
}

#[test]
fn test_two_step_admin_transfer_flow() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    let new_admin = Address::generate(&env);
    let unauthorized = Address::generate(&env);

    // Initial check
    assert_eq!(client.pending_admin(), None);
    assert_eq!(client.has_role(&Role::Admin, &authority), true);
    assert_eq!(client.has_role(&Role::Admin, &new_admin), false);

    // Non-admin cannot propose
    let err = client.try_propose_admin(&unauthorized, &new_admin);
    assert_eq!(err, Err(Ok(Error::NotAuthorized)));

    // Admin proposes new_admin
    client.propose_admin(&authority, &new_admin);
    assert_eq!(client.pending_admin(), Some(new_admin.clone()));

    // Unauthorized address cannot claim
    let err_claim = client.try_claim_admin(&unauthorized);
    assert_eq!(err_claim, Err(Ok(Error::NotPendingAuthority)));

    // New admin claims role
    client.claim_admin(&new_admin);

    // Transfer completed: new_admin is Admin, authority lost Admin, pending cleared
    assert_eq!(client.pending_admin(), None);
    assert_eq!(client.has_role(&Role::Admin, &new_admin), true);
    assert_eq!(client.has_role(&Role::Admin, &authority), false);
}

#[test]
fn test_proposal_vote_tally_snapshotting_prevents_double_vote_transfer() {
    use soroban_sdk::testutils::Ledger as _;

    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    let voter_a = Address::generate(&env);
    let wallet_b = Address::generate(&env);

    // At ledger 10, voter_a holds 100 voting weight
    env.ledger().set_sequence_number(10);
    client.checkpoint_balance(&authority, &voter_a, &100);
    assert_eq!(client.get_prior_votes(&voter_a, &10), 100);

    // At ledger 15, proposal is created -> snapshot_ledger is anchored at 15
    env.ledger().set_sequence_number(15);
    let proposal_id = client.propose(&authority, &3600);
    assert_eq!(client.proposal_snapshot_ledger(&proposal_id), 15);

    // voter_a casts vote -> tallies 100 votes from snapshot at ledger 15
    client.cast_vote(&voter_a, &proposal_id, &true);
    let snap = client.get_proposal_snapshot(&proposal_id);
    assert_eq!(snap.for_votes, 100);
    assert_eq!(snap.snapshot_ledger, 15);

    // voter_a cannot vote a second time directly
    assert_eq!(
        client.try_cast_vote(&voter_a, &proposal_id, &true),
        Err(Ok(Error::AlreadyVoted))
    );

    // At ledger 20 (after proposal snapshot ledger 15), voter_a moves tokens to wallet_b
    env.ledger().set_sequence_number(20);
    client.checkpoint_balance(&authority, &voter_a, &0);
    client.checkpoint_balance(&authority, &wallet_b, &100);

    // wallet_b had 0 weight at snapshot ledger 15 -> double-vote attempt is rejected!
    assert_eq!(
        client.try_cast_vote(&wallet_b, &proposal_id, &true),
        Err(Ok(Error::InsufficientVotingWeight))
    );
}
