use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotAuthorized = 1,
    InvalidParam = 2,
    AlreadyInitialized = 3,
    /// Refused to revoke the last `Admin`, which would freeze governance.
    LastAdmin = 4,
    /// The governor has not been initialised yet (required storage keys
    /// are missing).
    NotInitialized = 5,
    /// The governor is under an emergency pause; parameter changes are halted.
    ContractPaused = 6,
    /// `pause` was called while the governor was already paused.
    AlreadyPaused = 7,
    /// `unpause` was called while the governor was not paused.
    NotPaused = 8,
    /// A cross-contract call into `DripFactory` (`pause_factory`/
    /// `unpause_factory`) failed or was rejected by the factory.
    FactoryCallFailed = 9,
    /// No pending authority transfer to accept.
    NoPendingAuthority = 10,
    /// Caller is not the pending authority.
    NotPendingAuthority = 11,
    /// The WASM hash provided to `upgrade` is all zeros (invalid).
    InvalidWasmHash = 12,
}
