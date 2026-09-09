#![no_std]

//! PUSD Lending Pool - Pi collateral, PUSD borrow.
//! Over-collateralized lending with liquidation support.

use soroban_sdk::{contract, contractevent, contractimpl, contracttype, token, symbol_short, Address, Env, Vec};

const BPS_SCALE: i128 = 10_000;
const DECIMALS: i128 = 10_000_000; // 7 decimals for Stellar assets

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Position {
    pub collateral: i128, // Pi amount (7 decimals)
    pub debt: i128,       // PUSD amount (7 decimals)
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub admin: Address,
    pub pi_token: Address,
    pub pusd_token: Address,
    pub oracle: Address,
    pub max_ltv_bps: i128,      // e.g. 8000 = 80%
    pub liq_threshold_bps: i128, // e.g. 8500 = 85%
    pub liq_bonus_bps: i128,     // e.g. 500 = 5%
    pub paused: bool,
}

#[contractevent(topics = ["Deposit"])]
pub struct DepositEvent {
    #[topic]
    pub from: Address,
    pub amount: i128,
}

#[contractevent(topics = ["Withdraw"])]
pub struct WithdrawEvent {
    #[topic]
    pub to: Address,
    pub amount: i128,
}

#[contractevent(topics = ["Borrow"])]
pub struct BorrowEvent {
    #[topic]
    pub to: Address,
    pub amount: i128,
}

#[contractevent(topics = ["Repay"])]
pub struct RepayEvent {
    #[topic]
    pub from: Address,
    pub amount: i128,
}

#[contractevent(topics = ["Liquidate"])]
pub struct LiquidateEvent {
    #[topic]
    pub user: Address,
    pub liquidator: Address,
    pub repay: i128,
    pub total_seize: i128,
}

#[contract]
pub struct LendingPool;

#[contractimpl]
impl LendingPool {
    /// Initialize pool with Pi token, PUSD token, oracle, and config
    pub fn initialize(
        e: Env,
        admin: Address,
        pi_token: Address,
        pusd_token: Address,
        oracle: Address,
        max_ltv_bps: i128,
        liq_threshold_bps: i128,
        liq_bonus_bps: i128,
    ) {
        if e.storage().instance().has(&symbol_short!("config")) {
            panic!("already initialized");
        }
        if max_ltv_bps <= 0 || max_ltv_bps > BPS_SCALE {
            panic!("invalid max_ltv_bps");
        }
        if liq_threshold_bps <= max_ltv_bps || liq_threshold_bps > BPS_SCALE {
            panic!("invalid liq_threshold_bps");
        }
        if liq_bonus_bps < 0 || liq_bonus_bps > BPS_SCALE {
            panic!("invalid liq_bonus_bps");
        }
        let config = Config {
            admin: admin.clone(),
            pi_token: pi_token.clone(),
            pusd_token: pusd_token.clone(),
            oracle: oracle.clone(),
            max_ltv_bps,
            liq_threshold_bps,
            liq_bonus_bps,
            paused: false,
        };
        e.storage().instance().set(&symbol_short!("config"), &config);
    }

    fn get_config(e: &Env) -> Config {
        e.storage()
            .instance()
            .get(&symbol_short!("config"))
            .unwrap_or_else(|| panic!("not initialized"))
    }

    fn load_position(e: &Env, user: &Address) -> Position {
        e.storage()
            .persistent()
            .get(&(symbol_short!("pos"), user.clone()))
            .unwrap_or(Position {
                collateral: 0,
                debt: 0,
            })
    }

    fn save_position(e: &Env, user: &Address, position: &Position) {
        e.storage()
            .persistent()
            .set(&(symbol_short!("pos"), user.clone()), position);
        e.storage()
            .persistent()
            .extend_ttl(&(symbol_short!("pos"), user.clone()), 1000, 2000);
    }

    fn get_pi_price(e: &Env, oracle: &Address) -> i128 {
        e.invoke_contract::<i128>(oracle, &symbol_short!("get_price"), Vec::new(e))
    }

    /// Deposit Pi as collateral
    pub fn deposit(e: Env, from: Address, amount: i128) {
        from.require_auth();
        if amount <= 0 {
            panic!("amount must be positive");
        }
        let config = Self::get_config(&e);
        if config.paused {
            panic!("pool is paused");
        }
        let token_client = token::Client::new(&e, &config.pi_token);
        token_client.transfer(&from, &e.current_contract_address(), &amount);
        let mut pos = Self::load_position(&e, &from);
        pos.collateral += amount;
        Self::save_position(&e, &from, &pos);
        DepositEvent { from: from.clone(), amount }.publish(&e);
    }

    /// Withdraw Pi collateral (must maintain LTV)
    pub fn withdraw(e: Env, to: Address, amount: i128) {
        to.require_auth();
        if amount <= 0 {
            panic!("amount must be positive");
        }
        let config = Self::get_config(&e);
        if config.paused {
            panic!("pool is paused");
        }
        let mut pos = Self::load_position(&e, &to);
        if pos.collateral < amount {
            panic!("insufficient collateral");
        }
        pos.collateral -= amount;
        if pos.debt > 0 {
            let pi_price = Self::get_pi_price(&e, &config.oracle);
            let coll_val = pos.collateral * pi_price / DECIMALS;
            let max_debt = coll_val * config.max_ltv_bps / BPS_SCALE;
            if pos.debt > max_debt {
                panic!("would exceed max LTV");
            }
        }
        Self::save_position(&e, &to, &pos);
        let token_client = token::Client::new(&e, &config.pi_token);
        token_client.transfer(&e.current_contract_address(), &to, &amount);
        WithdrawEvent { to: to.clone(), amount }.publish(&e);
    }

    /// Borrow PUSD against Pi collateral
    pub fn borrow(e: Env, to: Address, amount: i128) {
        to.require_auth();
        if amount <= 0 {
            panic!("amount must be positive");
        }
        let config = Self::get_config(&e);
        if config.paused {
            panic!("pool is paused");
        }
        let pos = Self::load_position(&e, &to);
        let pi_price = Self::get_pi_price(&e, &config.oracle);
        let coll_val = pos.collateral * pi_price / DECIMALS;
        let new_debt = pos.debt + amount;
        let max_debt = coll_val * config.max_ltv_bps / BPS_SCALE;
        if new_debt > max_debt {
            panic!("exceeds max LTV");
        }
        let mut new_pos = pos;
        new_pos.debt = new_debt;
        Self::save_position(&e, &to, &new_pos);
        let token_client = token::Client::new(&e, &config.pusd_token);
        token_client.transfer(&e.current_contract_address(), &to, &amount);
        BorrowEvent { to: to.clone(), amount }.publish(&e);
    }

    /// Repay PUSD debt
    pub fn repay(e: Env, from: Address, amount: i128) {
        from.require_auth();
        if amount <= 0 {
            panic!("amount must be positive");
        }
        let config = Self::get_config(&e);
        if config.paused {
            panic!("pool is paused");
        }
        let mut pos = Self::load_position(&e, &from);
        if pos.debt < amount {
            panic!("repay exceeds debt");
        }
        let token_client = token::Client::new(&e, &config.pusd_token);
        token_client.transfer(&from, &e.current_contract_address(), &amount);
        pos.debt -= amount;
        Self::save_position(&e, &from, &pos);
        RepayEvent { from: from.clone(), amount }.publish(&e);
    }

    /// Liquidate undercollateralized position. Liquidator repays PUSD debt and receives Pi collateral + bonus
    pub fn liquidate(e: Env, liquidator: Address, user: Address, repay_amount: i128) {
        liquidator.require_auth();
        if repay_amount <= 0 {
            panic!("amount must be positive");
        }
        let config = Self::get_config(&e);
        if config.paused {
            panic!("pool is paused");
        }
        let pos = Self::load_position(&e, &user);
        if pos.debt == 0 {
            panic!("no debt to liquidate");
        }
        let pi_price = Self::get_pi_price(&e, &config.oracle);
        let coll_val = pos.collateral * pi_price / DECIMALS;
        let min_coll_val = pos.debt * BPS_SCALE / config.liq_threshold_bps;
        if coll_val >= min_coll_val {
            panic!("position not liquidatable");
        }
        let repay = repay_amount.min(pos.debt);
        let collateral_to_seize = repay * DECIMALS / pi_price;
        let bonus = collateral_to_seize * config.liq_bonus_bps / BPS_SCALE;
        let total_seize = collateral_to_seize + bonus;
        if total_seize > pos.collateral {
            panic!("insufficient collateral to seize");
        }
        let pusd_client = token::Client::new(&e, &config.pusd_token);
        pusd_client.transfer(&liquidator, &e.current_contract_address(), &repay);
        let pi_client = token::Client::new(&e, &config.pi_token);
        pi_client.transfer(&e.current_contract_address(), &liquidator, &total_seize);
        let mut new_pos = pos;
        new_pos.debt -= repay;
        new_pos.collateral -= total_seize;
        Self::save_position(&e, &user, &new_pos);
        LiquidateEvent {
            user: user.clone(),
            liquidator: liquidator.clone(),
            repay,
            total_seize,
        }
        .publish(&e);
    }

    /// Get position for user
    pub fn get_position(e: Env, user: Address) -> Position {
        Self::load_position(&e, &user)
    }

    /// Get config
    pub fn config(e: Env) -> Config {
        Self::get_config(&e)
    }

    /// Pause/unpause (admin only)
    pub fn set_paused(e: Env, admin: Address, paused: bool) {
        admin.require_auth();
        let mut config = Self::get_config(&e);
        if admin != config.admin {
            panic!("not admin");
        }
        config.paused = paused;
        e.storage().instance().set(&symbol_short!("config"), &config);
    }
}

#[cfg(test)]
mod test;
