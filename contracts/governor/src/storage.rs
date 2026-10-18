use soroban_sdk::{contracttype, Address, Env, Vec};

/// Proposal status.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProposalStatus {
    Pending = 0,
    Executed = 1,
    Cancelled = 2,
}

/// Historical token balance / voting weight checkpoint at a specific ledger sequence.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BalanceCheckpoint {
    /// Ledger sequence number when this balance checkpoint was recorded.
    pub ledger: u32,
    /// Token balance / voting weight at `ledger`.
    pub weight: u64,
}

/// Snapshot of a proposal's checkpointed vote tallies anchored at its `snapshot_ledger`.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoteTallySnapshot {
    pub proposal_id: u64,
    pub snapshot_ledger: u32,
    pub for_votes: u64,
    pub against_votes: u64,
    pub total_supply: u64,
}

/// Minimal proposal struct for governance proposals.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// Address of the proposal author.
    pub author: Address,
    /// Timestamp when the proposal was created.
    pub created_at: u64,
    /// Ledger sequence at which voter token balances are snapshotted for this proposal.
    pub snapshot_ledger: u32,
    /// Window (in seconds) within which the proposal must be executed after passing.
    pub execution_window_secs: u64,
    /// Current status of the proposal.
    pub status: ProposalStatus,
    /// Number of votes cast in favor.
    pub for_votes: u64,
    /// Number of votes cast against.
    pub against_votes: u64,
}

/// Protocol administration roles.
///
/// Each role gates a distinct slice of governor state, so independent wallets
/// can own fee policy and rate/duration bounds without sharing a single
/// all-powerful key:
///
/// - `Admin`       — grant and revoke roles (including `Admin` itself).
/// - `FeeManager`  — `set_fee_bps`, `set_fee_recipient`.
/// - `RateManager` — `set_max_rate`, `set_min_duration`.
/// - `Pauser`      — `governor_pause`, `governor_unpause`, `pause_factory`, `unpause_factory`.
///
/// A role may be held by any number of accounts, and one account may hold any
/// combination of roles.
///
/// The enum stays governor-specific because role names are part of this
/// contract's ABI. Shared grant/revoke mechanics live in `drip_common::rbac`;
/// see ADR-008 for why the role enums are intentionally not shared.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Admin,
    FeeManager,
    RateManager,
    Pauser,
}

/// Composite key identifying a single (role, account) grant.
///
/// Soroban `contracttype` enum variants carry at most one payload, so the
/// (role, account) pair is wrapped in this struct rather than expressed as a
/// two-field `DataKey` variant.
#[contracttype]
#[derive(Clone)]
pub struct RoleKey {
    pub role: Role,
    pub account: Address,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Fee in basis points (e.g. 30 = 0.3%)
    FeeBps,
    /// Address that receives protocol fees
    FeeRecipient,
    /// Minimum allowed stream duration in seconds
    MinDurationSeconds,
    /// Maximum rate per second in stroops
    MaxRatePerSecond,
    /// Maximum allowed stream duration in seconds
    MaxDurationSeconds,
    /// The DripFactory contract this governor controls
    FactoryAddress,
    /// Seconds a stream must remain paused before `DripStream::force_cancel`
    /// becomes callable by the recipient.
    ForceCancelPauseSecs,
    /// Presence marks that `account` holds `role`. The stored value is an
    /// unused `bool`; membership is expressed entirely by the key existing.
    Role(RoleKey),
    /// Number of accounts currently holding `Role::Admin`. Tracked so the last
    /// admin can never be revoked, which would freeze governance permanently.
    AdminCount,
    /// Emergency-pause flag. When `true`, parameter writes are blocked.
    Paused,
    /// Index of all accounts currently holding a given role.
    /// Maintained alongside `grant`/`revoke` so role membership can be
    /// enumerated on-chain without replaying every event from genesis.
    RoleMembers(Role),
    /// Pending authority address for 2-step transfer.
    /// Set by `propose_authority`, consumed by `accept_authority`.
    PendingAuthority,
    /// The admin who proposed the authority transfer.
    /// Used to revoke their Admin role when the transfer is accepted.
    PendingAuthorityProposer,
    /// Governance proposal by ID.
    Proposal(u64),
    /// Proposal counter for generating proposal IDs.
    ProposalCounter,
    /// Minimum quorum threshold in basis points (e.g. 2000 = 20%).
    MinQuorumBps,
    /// Optional total governance supply override; defaults to AdminCount if not set.
    GovernanceSupply,
    /// Historical token balance / voting weight checkpoints per account.
    VoteCheckpoints(Address),
    /// Whether token balance checkpointing is active on this governor.
    CheckpointingEnabled,
    /// Snapshot ledger sequence recorded when proposal `u64` was created.
    ProposalSnapshotLedger(u64),
    /// Vote tally snapshot for proposal `u64` anchored at its snapshot ledger.
    ProposalVoteSnapshot(u64),
    /// Receipt recording the voting weight cast by `(proposal_id, voter)` to prevent double-voting.
    VoterReceipt(u64, Address),
}

/// Records a token balance checkpoint for `account` at the current ledger sequence.
pub fn record_checkpoint(env: &Env, account: &Address, weight: u64) {
    let ledger = env.ledger().sequence();
    record_checkpoint_at(env, account, ledger, weight);
}

/// Records a token balance checkpoint for `account` at `ledger`.
pub fn record_checkpoint_at(env: &Env, account: &Address, ledger: u32, weight: u64) {
    env.storage()
        .instance()
        .set(&DataKey::CheckpointingEnabled, &true);

    let key = DataKey::VoteCheckpoints(account.clone());
    let mut checkpoints: Vec<BalanceCheckpoint> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));

    let len = checkpoints.len();
    if len > 0 {
        let last = checkpoints.get(len - 1).unwrap();
        if last.ledger >= ledger {
            checkpoints.set(len - 1, BalanceCheckpoint { ledger, weight });
            env.storage().persistent().set(&key, &checkpoints);
            drip_common::ttl::bump_persistent(env, &key);
            return;
        }
    }

    checkpoints.push_back(BalanceCheckpoint { ledger, weight });
    env.storage().persistent().set(&key, &checkpoints);
    drip_common::ttl::bump_persistent(env, &key);
}

/// Looks up the checkpointed voting weight for `account` at or before `snapshot_ledger`.
///
/// Returns `Some(weight)` if checkpointing is enabled or `account` has checkpoints,
/// where `weight` is `0` if all checkpoints for `account` were recorded strictly
/// after `snapshot_ledger` (preventing double-voting via post-snapshot token transfers).
/// Returns `None` if checkpointing has not been used at all.
pub fn get_checkpoint_weight(env: &Env, account: &Address, snapshot_ledger: u32) -> Option<u64> {
    let key = DataKey::VoteCheckpoints(account.clone());
    let checkpoints: Option<Vec<BalanceCheckpoint>> = env.storage().persistent().get(&key);

    if let Some(cps) = checkpoints {
        let mut matched_weight: u64 = 0;
        for cp in cps.iter() {
            if cp.ledger <= snapshot_ledger {
                matched_weight = cp.weight;
            } else {
                break;
            }
        }
        return Some(matched_weight);
    }

    let enabled: bool = env
        .storage()
        .instance()
        .get(&DataKey::CheckpointingEnabled)
        .unwrap_or(false);
    if enabled {
        Some(0)
    } else {
        None
    }
}

/// Stores the snapshot ledger for `proposal_id`.
pub fn set_proposal_snapshot_ledger(env: &Env, proposal_id: u64, snapshot_ledger: u32) {
    env.storage()
        .instance()
        .set(&DataKey::ProposalSnapshotLedger(proposal_id), &snapshot_ledger);
}

/// Reads the snapshot ledger for `proposal_id`.
pub fn get_proposal_snapshot_ledger(env: &Env, proposal_id: u64) -> Option<u32> {
    env.storage()
        .instance()
        .get(&DataKey::ProposalSnapshotLedger(proposal_id))
}

/// Saves the proposal vote tally snapshot anchored at the proposal's snapshot ledger.
pub fn save_vote_tally_snapshot(env: &Env, snapshot: &VoteTallySnapshot) {
    env.storage()
        .instance()
        .set(&DataKey::ProposalVoteSnapshot(snapshot.proposal_id), snapshot);
}

/// Loads the proposal vote tally snapshot for `proposal_id`.
pub fn get_vote_tally_snapshot(env: &Env, proposal_id: u64) -> Option<VoteTallySnapshot> {
    env.storage()
        .instance()
        .get(&DataKey::ProposalVoteSnapshot(proposal_id))
}

/// Returns `true` if `voter` has already voted on `proposal_id`.
pub fn has_voted(env: &Env, proposal_id: u64, voter: &Address) -> bool {
    env.storage()
        .instance()
        .has(&DataKey::VoterReceipt(proposal_id, voter.clone()))
}

/// Records that `voter` cast a vote with `weight` on `proposal_id`.
pub fn record_voter_receipt(env: &Env, proposal_id: u64, voter: &Address, weight: u64) {
    env.storage()
        .instance()
        .set(&DataKey::VoterReceipt(proposal_id, voter.clone()), &weight);
}

