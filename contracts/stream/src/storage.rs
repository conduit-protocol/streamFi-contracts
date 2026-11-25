use soroban_sdk::{contracttype, Address, Env, String};

// Bit-flags packed into `StreamInfo::flags`. Kept `pub` so cross-crate
// regression tests (e.g. `tests/audit_round_2_regression.rs::pause_resume_*`)
// and the `info().is_paused()`/`is_cancelled()`/`is_clawback_enabled()` getters
// can use them, but marked `#[doc(hidden)]` to keep the rustdoc contract API
// surface clean. Off-chain callers should use the `is_*()` getters rather than
// reading the bit values directly.
#[doc(hidden)]
pub const FLAG_PAUSED: u32 = 1;
#[doc(hidden)]
pub const FLAG_CLAWBACK_ENABLED: u32 = 1 << 1;
#[doc(hidden)]
pub const FLAG_CANCELLED: u32 = 1 << 2;
#[doc(hidden)]
pub const FLAG_REGULATED_ASSET: u32 = 1 << 3;

/// Current storage layout version for this contract.
/// Bump this and add an explicit migration check whenever a future
/// upgrade changes the shape of persisted `StreamInfo`/`Config` data.
pub const CURRENT_STORAGE_VERSION: u32 = 1;

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Single-key representation of all stream fields.
    /// Consolidates all stream metadata into one storage read instead of eleven
    /// legacy per-field keys (superseded on first `save()` of pre-consolidation streams).
    Config,
    /// Immutable/low-churn stream metadata (#942).
    Metadata,
    /// Frequently-mutated stream state (#942).
    State,
    /// Lock for re-entrancy protection and concurrency control.
    Guard,
    /// Storage layout version, written once at `initialize()`.
    /// Future contract upgrades must check this before assuming the
    /// persisted `StreamInfo`/`Config` layout is compatible.
    StorageVersion,
    /// Optional operator address delegated by the sender.
    ///
    /// When set, the operator can perform sender-level actions (pause,
    /// cancel, clawback, top_up, extend_duration) on behalf of the sender.
    /// Absent key means no operator has been delegated.
    Operator,
    /// Seconds a stream must remain paused before `force_cancel` becomes
    /// callable by the recipient. Set once at `initialize()` from the
    /// value `DripFactory::create_stream` read from `GovernorConfig` at
    /// deploy time (governance-configurable per deployment; see
    /// `DripGovernor::set_force_cancel_pause_threshold`). Stored on the
    /// stream itself — rather than read cross-contract on every
    /// `force_cancel` call — because ADR-001 keeps this contract's hot
    /// path free of cross-contract calls; a stream deployed directly
    /// (bypassing the factory) falls back to the historical 30-day default.
    ForceCancelPauseThresholdSecs,
    /// Optional split configuration dividing claims between recipient and secondary recipient.
    SplitConfig,
    /// Optional cliff configuration with upfront unlock amount (Issue #719).
    CliffConfig,
    /// Configurable minimum interval in seconds between consecutive withdrawals (Issue #693).
    MinWithdrawalInterval,
    /// Timestamp of the last withdrawal.
    LastWithdrawalTime,
    /// Optional accrual checkpoint created by `change_rate`.
    ///
    /// Kept outside `StreamInfo` so adding rate changes does not alter the
    /// serialized `Config` layout of already-deployed streams.
    RateCheckpoint,
    /// Emergency pause flag adhering to drip_common::pause (Issue #699).
    EmergencyPaused,
    /// Immutable stream configuration in instance storage (Issue #702).
    ImmutableConfig,
    /// Milestone flag indicating the cliff timestamp was passed and event emitted (Issue #676).
    CliffReached,
    /// Track whether cliff amount has been claimed on first withdrawal after cliff (Issue #752).
    CliffClaimed,
    /// Milestone flag indicating the stream has fully settled and completion event emitted (Issue #676).
    StreamCompleted,
    /// Stream cancellation configuration (Issue #685).
    StreamConfig,
    /// Optional stream title updated post-creation by sender.
    /// Allows sender to rename or describe the stream.
    StreamTitle,
    /// Optional reference tag (e.g. invoice, order, or external ID).
    /// Allows sender to link the stream to external systems.
    ReferenceTag,
    /// Optional human-readable description set at creation (#845).
    StreamDescription,
    /// Optional metadata URI for stream invoices (Issue #748).
    InvoiceMetadataUri,
    /// Optional metadata URI for stream milestones (Issue #748).
    MilestoneMetadataUri,
    /// Optional invoice/reference metadata URI set at creation (Issue #923).
    MetadataUri,
    /// Separately funded token pool used to reimburse stream keepers (#840).
    KeeperRewardPool,
    /// Last ledger on which a keeper reward was paid.
    LastKeeperPingLedger,
    /// Optional address that receives the recipient's stream payouts (#836).
    AutoForwardAddress,
    /// Total amount funded into the stream across its lifetime (#837).
    CumulativeDeposited,
    /// Optional secondary backup recovery address in stream state (Issue #810).
    RecoveryAddress,
    /// Timelock timestamp after which a recovery claim may be executed (Issue #810).
    RecoveryClaimableAt,
    /// Optional flag allowing keepers or senders to auto-settle the stream upon stream end (Issue #809).
    AutoSettle,
    /// Persistent storage record of stream cancellation reason and metadata (Issue #755).
    CancelledRecord,
    /// Funder approved by the sender to call `top_up_by_funder` (#933).
    ApprovedFunder(Address),
    /// Time-bounded emergency cancellation delegate configured by the sender (#886).
    CancelDelegate,
    /// Delegate the recipient allowed to claim on its behalf via `claim_for` (#878).
    ClaimDelegate,
    /// Compact bitfield storing boolean stream status flags (#882).
    StatusFlags,
    /// Optional non-linear vesting curve configuration (#883).
    VestingCurve,
    RecipientProposal,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipientProposal {
    pub current_recipient: Address,
    pub new_recipient: Address,
    pub proposed_at: u64,
}

/// Claim delegate granted by a recipient (#878). The grant is tied to the
/// recipient that made it, so it stops working if the recipient changes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimDelegateConfig {
    pub recipient: Address,
    pub delegate: Address,
}

/// Emergency cancellation delegate with a time-bounded expiration timestamp (#886).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelDelegateConfig {
    pub delegate: Address,
    pub expires_at: u64,
}

/// Immutable stream configuration stored in instance storage for cheaper reads (Issue #702).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImmutableConfig {
    pub start_time: u64,
    pub stop_time: u64,
    pub token: Address,
    pub rate_per_second: i128,
    pub sender: Address,
    pub recipient: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamKeeperRewardPool {
    pub token: Address,
    pub reward_per_ping: i128,
    pub balance: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SplitConfig {
    pub secondary_recipient: Address,
    pub split_bps: u32,
}

/// Stream configuration including cliff and upfront unlock parameters (Issue #719).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliffConfig {
    pub cliff_time: u64,
    pub cliff_unlock_amount: i128,
}

/// Accrued-value checkpoint used to make rate changes prospective.
///
/// `accrued` is the total amount earned at `at`. From that point onward
/// streaming uses the current `StreamInfo::rate_per_second`. For a pending
/// stream (or a stream still before its cliff), `at` may be in the future;
/// no value is considered streamed until that timestamp is reached.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateCheckpoint {
    pub accrued: i128,
    pub at: u64,
}

/// Stream configuration including cancelability and optional reconciliation memo (Issues #685, #803, #938).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamConfig {
    pub is_cancelable: bool,
    pub memo: Option<String>,
    /// Optional cliff unlock percentage in basis points (Issue #938).
    /// When set, this percentage of the vested amount unlocks immediately at cliff.
    /// Valid range: 0-10000 (0% to 100%). E.g., 1000 means 10% unlocks at cliff.
    pub cliff_unlock_bps: Option<u32>,
}

/// Operational status of a stream.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamStatus {
    Pending = 0,
    Active = 1,
    Paused = 2,
    Cancelled = 3,
    Completed = 4,
}

/// Consolidated state summary for a stream.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamSummary {
    pub sender: Address,
    pub recipient: Address,
    pub token: Address,
    pub total_amount: i128,
    pub streamed_amount: i128,
    pub status: StreamStatus,
    pub start_time: u64,
    pub stop_time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamMetadata {
    pub sender: Address,
    pub token: Address,
    pub start_time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutableStreamState {
    pub recipient: Address,
    pub rate_per_second: i128,
    pub end_time: u64,
    pub withdrawn: i128,
    pub paused_at: u64,
    pub flags: u32,
    pub event_sequence: u64,
    pub cumulative_fees: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamInfo {
    pub sender: Address,
    pub recipient: Address,
    pub token: Address,
    pub rate_per_second: i128,
    pub start_time: u64,
    pub end_time: u64,
    pub withdrawn: i128,
    pub paused_at: u64,
    pub flags: u32,
    pub event_sequence: u64,
    pub cumulative_fees: i128,
}

impl StreamInfo {
    /// True when the `FLAG_PAUSED` bit (0x01) is set in `flags`.
    pub fn is_paused(&self) -> bool {
        (self.flags & FLAG_PAUSED) != 0
    }

    /// True when the `FLAG_CANCELLED` bit (0x04) is set in `flags`.
    pub fn is_cancelled(&self) -> bool {
        (self.flags & FLAG_CANCELLED) != 0
    }

    /// True when the `FLAG_CLAWBACK_ENABLED` bit (0x02) is set in `flags`.
    pub fn is_clawback_enabled(&self) -> bool {
        (self.flags & FLAG_CLAWBACK_ENABLED) != 0
    }

    /// True when the `FLAG_REGULATED_ASSET` bit (0x08) is set in `flags` (Issue #781).
    pub fn is_regulated_asset(&self) -> bool {
        (self.flags & FLAG_REGULATED_ASSET) != 0
    }

    /// Mark the stream as cancelled, setting the correct flag bit.
    pub fn mark_cancelled(&mut self) {
        self.flags |= FLAG_CANCELLED;
    }

    /// Mark the stream as using a regulated asset, setting the correct flag bit (Issue #781).
    pub fn mark_regulated_asset(&mut self) {
        self.flags |= FLAG_REGULATED_ASSET;
    }

    /// Explicit cumulative payout counter (Issue #936).
    ///
    /// `withdrawn` is persisted independently from the contract's live token
    /// balance; exposing it by name avoids reconstructing historical events.
    pub fn total_withdrawn(&self) -> i128 {
        self.withdrawn
    }

    /// True when all deposited stream funds have been completely withdrawn or stream is ended and fully drained (Issue #753).
    pub fn is_depleted(&self) -> bool {
        if self.is_cancelled() {
            true
        } else if self.end_time > 0 && self.rate_per_second > 0 {
            let duration = self.end_time.saturating_sub(self.start_time) as i128;
            let total = self.rate_per_second.saturating_mul(duration);
            self.withdrawn >= total
        } else {
            false
        }
    }

    /// True once a bounded stream's schedule has completed at `current_time` (Issue #927).
    ///
    /// This is a time check only: it says the stream has reached `end_time`,
    /// not that every token has been withdrawn. Use [`Self::is_depleted`] for
    /// the funding question. Open-ended streams (`end_time == 0`) have no
    /// scheduled completion and are never fully vested.
    pub fn is_fully_vested(&self, current_time: u64) -> bool {
        self.end_time > 0 && current_time >= self.end_time
    }

    /// Calculate stream age in seconds from start_time up to current_time (Issue #808).
    pub fn elapsed_seconds(&self, current_time: u64) -> u64 {
        if current_time <= self.start_time {
            return 0;
        }
        let effective_end = if self.end_time > 0 {
            current_time.min(self.end_time)
        } else {
            current_time
        };
        effective_end.saturating_sub(self.start_time)
    }

    /// Check if the stream is in a cancellable state (Issue #937).
    ///
    /// A stream can be cancelled if it has not already been cancelled.
    /// The caller authorization (sender vs recipient permissions) is checked
    /// at the contract method level.
    pub fn is_in_cancellable_state(&self) -> bool {
        !self.is_cancelled()
    }
}

/// Alias for `StreamInfo` representing persisted stream state (Issue #808).
pub type StreamState = StreamInfo;

/// Record of stream cancellation stored in persistent storage (Issue #755).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelledStreamRecord {
    pub cancellation_reason: u32,
    pub cancelled_at: u64,
    pub cancelled_by: Address,
    pub refund_amount: i128,
}

pub fn set_cancelled_record(env: &Env, record: &CancelledStreamRecord) {
    let key = DataKey::CancelledRecord;
    env.storage().persistent().set(&key, record);
    drip_common::ttl::bump_persistent(env, &key);
}

pub fn get_cancelled_record(env: &Env) -> Option<CancelledStreamRecord> {
    env.storage().persistent().get(&DataKey::CancelledRecord)
}

/// Compact 8-bit bitfield packing boolean stream status flags to minimize
/// Soroban persistent storage footprint (#882).
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct StreamFlags(pub u8);

impl StreamFlags {
    pub const PAUSED: u8 = 1 << 0;
    pub const CLIFF_CLAIMED: u8 = 1 << 1;
    pub const CANCELLED: u8 = 1 << 2;
    pub const DEPLETED: u8 = 1 << 3;

    pub const fn new() -> Self {
        Self(0)
    }

    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn is_paused(self) -> bool {
        (self.0 & Self::PAUSED) != 0
    }

    pub fn set_paused(&mut self, value: bool) {
        if value {
            self.0 |= Self::PAUSED;
        } else {
            self.0 &= !Self::PAUSED;
        }
    }

    pub const fn is_cliff_claimed(self) -> bool {
        (self.0 & Self::CLIFF_CLAIMED) != 0
    }

    pub fn set_cliff_claimed(&mut self, value: bool) {
        if value {
            self.0 |= Self::CLIFF_CLAIMED;
        } else {
            self.0 &= !Self::CLIFF_CLAIMED;
        }
    }

    pub const fn is_cancelled(self) -> bool {
        (self.0 & Self::CANCELLED) != 0
    }

    pub fn set_cancelled(&mut self, value: bool) {
        if value {
            self.0 |= Self::CANCELLED;
        } else {
            self.0 &= !Self::CANCELLED;
        }
    }

    pub const fn is_depleted(self) -> bool {
        (self.0 & Self::DEPLETED) != 0
    }

    pub fn set_depleted(&mut self, value: bool) {
        if value {
            self.0 |= Self::DEPLETED;
        } else {
            self.0 &= !Self::DEPLETED;
        }
    }
}

pub fn set_status_flags(env: &Env, flags: StreamFlags) {
    let key = DataKey::StatusFlags;
    let raw: u32 = u32::from(flags.bits());
    env.storage().persistent().set(&key, &raw);
    drip_common::ttl::bump_persistent(env, &key);
}

pub fn get_status_flags(env: &Env) -> StreamFlags {
    let raw: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::StatusFlags)
        .unwrap_or(0);
    StreamFlags::from_bits(raw as u8)
}
