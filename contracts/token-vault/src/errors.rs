use soroban_sdk::{contracterror, Env};

#[contracterror]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Error {
    InvalidAmount = 1,
    ArithmeticOverflow = 2,
    LimitExceeded = 3,
    NotAuthorized = 4,
}

impl From<Error> for soroban_sdk::Symbol {
    fn from(e: Error) -> Self {
        match e {
            Error::InvalidAmount => soroban_sdk::Symbol::short("IA"),
            Error::ArithmeticOverflow => soroban_sdk::Symbol::short("AO"),
            Error::LimitExceeded => soroban_sdk::Symbol::short("LE"),
            Error::NotAuthorized => soroban_sdk::Symbol::short("NA"),
        }
    }
}
