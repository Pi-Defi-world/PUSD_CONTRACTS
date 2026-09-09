#![no_std]

//! Pi price oracle - stores Pi/USD price. Backend pushes from Oracle API.
//! Price is stored as integer: price * 10^7 (7 decimals, e.g. 0.25 USD = 2_500_000)

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleData {
    pub price: i128,     // Pi price in USD * 10^7
    pub updated_at: u64, // Ledger timestamp
}

#[contract]
pub struct PriceOracle;

#[contractimpl]
impl PriceOracle {
    /// Initialize the oracle with admin address
    pub fn initialize(e: Env, admin: Address) {
        if e.storage().instance().has(&soroban_sdk::symbol_short!("admin")) {
            panic!("already initialized");
        }
        e.storage().instance().set(&soroban_sdk::symbol_short!("admin"), &admin);
    }

    /// Set Pi price (admin only). Price in USD with 7 decimals (e.g. 0.25 = 2_500_000)
    pub fn set_price(e: Env, admin: Address, price: i128) {
        admin.require_auth();
        let stored_admin: Address = e
            .storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("admin"))
            .unwrap_or_else(|| panic!("not initialized"));
        if admin != stored_admin {
            panic!("not admin");
        }
        if price <= 0 {
            panic!("price must be positive");
        }
        let data = OracleData {
            price,
            updated_at: e.ledger().timestamp(),
        };
        e.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("pi_price"), &data);
        e.storage().instance().extend_ttl(1000, 2000);
    }

    /// Get Pi price (returns price * 10^7)
    pub fn get_price(e: Env) -> i128 {
        e.storage()
            .instance()
            .get::<_, OracleData>(&soroban_sdk::symbol_short!("pi_price"))
            .unwrap_or_else(|| panic!("price not set"))
            .price
    }

    /// Get full oracle data
    pub fn get_oracle_data(e: Env) -> OracleData {
        e.storage()
            .instance()
            .get::<_, OracleData>(&soroban_sdk::symbol_short!("pi_price"))
            .unwrap_or_else(|| panic!("price not set"))
    }

    pub fn admin(e: Env) -> Address {
        e.storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("admin"))
            .unwrap_or_else(|| panic!("not initialized"))
    }
}

#[cfg(test)]
mod test;
