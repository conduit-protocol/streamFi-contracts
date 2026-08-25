use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Error {
    InvalidAmount = 1,
    ArithmeticOverflow = 2,
    LimitExceeded = 3,
    NotAuthorized = 4,
    /// `deposit`, `withdraw`, or `set_limit` was called while the vault is
    /// under an emergency pause.
    ContractPaused = 5,
    /// `pause` was called while the vault was already paused.
    AlreadyPaused = 6,
    /// `unpause` was called while the vault was not paused.
    NotPaused = 7,
}
