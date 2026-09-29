use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// `init` was called before any other admin-side call.
    NotInitialized = 1,
    /// `init` was called more than once.
    AlreadyInitialized = 2,
    /// `get_twap` or `submit_observation` was called for an asset that has
    /// no recorded `AssetConfig`.
    AssetNotConfigured = 3,
    /// `submit_observation` was called by an address that is not in the
    /// asset's whitelisted oracle set.
    OracleNotWhitelisted = 4,
    /// Sampled timestamp is older than the latest stored observation (not
    /// monotonically increasing) or further into the future than the
    /// configured clock-drift tolerance allows.
    TimestampInvalid = 5,
    /// The most recent observation in the buffer is older than the asset's
    /// `max_staleness` window. Reader must wait for a fresh push or treat
    /// the feed as offline.
    PriceStale = 6,
    /// Not enough buffered observations sit inside the configured
    /// `window_size` to reconstruct a time-weighted average.
    InsufficientSamples = 7,
    /// Internal i128 arithmetic overflowed while computing the weighted sum
    /// or quotient.
    ArithmeticOverflow = 8,
    /// Sampled price is zero or negative; rejects an obviously broken feed.
    InvalidPrice = 9,
    /// `configure_asset` was given an invalid parameter set
    /// (`decimals > MAX_DECIMALS`, `window_size == 0`, `max_staleness == 0`,
    /// `window_size > max_staleness`, etc.).
    InvalidConfig = 10,
    /// Caller is not the admin configured in instance storage. Surfaces a
    /// distinct variant so the contract panics alone from
    /// `require_auth()` and admin-mismatch can both be observed cleanly.
    NotAuthorized = 11,
    /// Price feed not found for requested pair.
    PriceNotFound = 12,
}
