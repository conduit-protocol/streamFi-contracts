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
}
