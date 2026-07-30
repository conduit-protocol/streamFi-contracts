use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Error {
    InvalidAmount = 1,
    ArithmeticOverflow = 2,
    LimitExceeded = 3,
    NotAuthorized = 4,
}
