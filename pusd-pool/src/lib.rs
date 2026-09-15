#![no_std]

//! PUSD lending pool: request-based submit, reserves (Pi collateral, PUSD borrow),
//! health factor, interest accrual. Compatible with Pi/Soroban.

use soroban_sdk::{
    contract, contractevent, contractimpl, contracttype, token, symbol_short, Address, Env, Map,
    Vec,
};

const SCALAR_7: i128 = 10_000_000;
const SCALAR_12: i128 = 1_000_000_000_000;
const DECIMALS: i128 = 10_000_000; // 7 decimals for Stellar assets
const SECONDS_PER_YEAR: i128 = 31_536_000;

// ----- Pool config -----

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolConfig {
    pub admin: Address,
    pub oracle: Address,
    pub min_collateral: i128,
    pub bstop_rate: u32,   // 7 decimals, 0 = no backstop
    pub status: u32,      // 0=admin_active, 1=active, 2+=restricted
    pub max_positions: u32,
}

// ----- Reserve config (per asset) -----

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReserveConfig {
    pub index: u32,
    pub decimals: u32,
    pub c_factor: u32, // collateral factor, 7 decimals (e.g. 0.8e7 = 80%)
    pub l_factor: u32, // liability factor, 7 decimals (e.g. 0.9e7 = 90%)
    pub interest_rate: u32, // annual borrow rate, 7 decimals (e.g. 500_000 = 5%)
    pub enabled: bool,
}

// ----- Reserve data (accrual state) -----

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReserveData {
    pub b_rate: i128,   // b_token to underlying, 12 decimals
    pub d_rate: i128,   // d_token to underlying, 12 decimals
    pub b_supply: i128, // total b_tokens (supply side)
    pub d_supply: i128, // total d_tokens (borrow side)
    pub last_time: u64,
}

// ----- Request types -----

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub request_type: u32, // 0=Supply, 1=Withdraw, 2=SupplyCollateral, 3=WithdrawCollateral, 4=Borrow, 5=Repay
    pub address: Address,  // asset address
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserReservePosition {
    pub collateral: i128, // b_token balance
    pub liability: i128,  // d_token balance
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Positions {
    pub collateral: Map<u32, i128>,
    pub liabilities: Map<u32, i128>,
}

#[contract]
pub struct PusdPool;

// ----- Events (for off-chain indexing / read-models) -----

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestKind {
    Supply = 0,
    Withdraw = 1,
    SupplyCollateral = 2,
    WithdrawCollateral = 3,
    Borrow = 4,
    Repay = 5,
}

#[contractevent(topics = ["Submit"])]
pub struct SubmitEvent {
    #[topic]
    pub from: Address,
    pub spender: Address,
    pub to: Address,
    pub request_count: u32,
}

#[contractevent(topics = ["Request"])]
pub struct RequestEvent {
    #[topic]
    pub from: Address,
    pub kind: u32,
    pub asset: Address,
    pub amount: i128,
}

#[contractevent(topics = ["Liquidate"])]
pub struct LiquidateEvent {
    #[topic]
    pub user: Address,
    pub liquidator: Address,
    pub repay_underlying: i128,
    pub seize_underlying: i128,
}

#[contractimpl]
impl PusdPool {
    /// Initialize pool with admin, oracle, and two reserves: Pi (index 0), PUSD (index 1)
    pub fn initialize(
        e: Env,
        admin: Address,
        oracle: Address,
        pi_token: Address,
        pusd_token: Address,
        backstop_take_rate: u32,
        max_positions: u32,
        min_collateral: i128,
    ) {
        if e.storage().instance().has(&symbol_short!("config")) {
            panic!("already initialized");
        }
        let config = PoolConfig {
            admin: admin.clone(),
            oracle,
            min_collateral,
            bstop_rate: backstop_take_rate,
            status: 1, // active
            max_positions,
        };
        e.storage().instance().set(&symbol_short!("config"), &config);

        let mut res_list = Vec::new(&e);
        res_list.push_back(pi_token.clone());
        res_list.push_back(pusd_token.clone());
        e.storage().instance().set(&symbol_short!("res_list"), &res_list);

        // Reserve 0: Pi - collateral only (c_factor 0.8, l_factor 0)
        let pi_config = ReserveConfig {
            index: 0,
            decimals: 7,
            c_factor: 8_000_000, // 80%
            l_factor: 0,
            interest_rate: 0, // Pi is collateral-only, not borrowable
            enabled: true,
        };
        let pi_data = ReserveData {
            b_rate: SCALAR_12,
            d_rate: SCALAR_12,
            b_supply: 0,
            d_supply: 0,
            last_time: e.ledger().timestamp(),
        };
        e.storage().instance().set(&(symbol_short!("res_cfg"), pi_token.clone()), &pi_config);
        e.storage().instance().set(&(symbol_short!("res_data"), pi_token.clone()), &pi_data);

        // Reserve 1: PUSD - borrowable (c_factor 0, l_factor 0.9)
        let pusd_config = ReserveConfig {
            index: 1,
            decimals: 7,
            c_factor: 0,
            l_factor: 9_000_000, // 90%
            interest_rate: 500_000, // 5% annual borrow rate
            enabled: true,
        };
        let pusd_data = ReserveData {
            b_rate: SCALAR_12,
            d_rate: SCALAR_12,
            b_supply: 0,
            d_supply: 0,
            last_time: e.ledger().timestamp(),
        };
        e.storage().instance().set(&(symbol_short!("res_cfg"), pusd_token.clone()), &pusd_config);
        e.storage().instance().set(&(symbol_short!("res_data"), pusd_token), &pusd_data);

        e.storage().instance().extend_ttl(1000, 2000);
    }

    fn load_config(e: &Env) -> PoolConfig {
        e.storage()
            .instance()
            .get(&symbol_short!("config"))
            .unwrap_or_else(|| panic!("not initialized"))
    }

    fn get_res_list(e: &Env) -> Vec<Address> {
        e.storage()
            .instance()
            .get(&symbol_short!("res_list"))
            .unwrap_or_else(|| panic!("no res list"))
    }

    fn get_reserve_config(e: &Env, asset: &Address) -> ReserveConfig {
        e.storage()
            .instance()
            .get(&(symbol_short!("res_cfg"), asset.clone()))
            .unwrap_or_else(|| panic!("reserve not found"))
    }

    fn raw_reserve_data(e: &Env, asset: &Address) -> ReserveData {
        e.storage()
            .instance()
            .get(&(symbol_short!("res_data"), asset.clone()))
            .unwrap_or_else(|| panic!("reserve data not found"))
    }

    /// Accrue interest for a reserve, advancing `b_rate` (supply) and `d_rate`
    /// (borrow) based on elapsed time since `last_time`. Idempotent within a
    /// ledger (dt == 0 is a no-op). 4.1.
    fn accrue_reserve(e: &Env, asset: &Address) {
        let mut data = Self::raw_reserve_data(e, asset);
        let rcfg = Self::get_reserve_config(e, asset);
        let now = e.ledger().timestamp();
        if now <= data.last_time {
            return;
        }
        let dt = now - data.last_time;

        let rate = i128::from(rcfg.interest_rate);
        if rate == 0 {
            data.last_time = now;
            Self::set_reserve_data(e, asset, &data);
            return;
        }

        // Borrow side accrues at the full annual rate.
        let accrued = rate * (dt as i128) / SECONDS_PER_YEAR; // 7-decimal fraction
        let d_growth = SCALAR_7 + accrued;
        data.d_rate = data.d_rate * d_growth / SCALAR_7;

        // Supply side accrues at utilization * borrow rate (reserve factor omitted for simplicity).
        let util = if data.b_supply > 0 {
            data.d_supply * SCALAR_7 / data.b_supply
        } else {
            0
        };
        let s_accrued = accrued * util / SCALAR_7;
        let b_growth = SCALAR_7 + s_accrued;
        data.b_rate = data.b_rate * b_growth / SCALAR_7;

        data.last_time = now;
        Self::set_reserve_data(e, asset, &data);
    }

    fn get_reserve_data(e: &Env, asset: &Address) -> ReserveData {
        Self::accrue_reserve(e, asset);
        Self::raw_reserve_data(e, asset)
    }

    fn set_reserve_data(e: &Env, asset: &Address, data: &ReserveData) {
        e.storage()
            .instance()
            .set(&(symbol_short!("res_data"), asset.clone()), data);
    }

    fn load_user_position(e: &Env, user: &Address, reserve_index: u32) -> (i128, i128) {
        let key = (symbol_short!("pos"), user.clone(), reserve_index);
        let stored: Option<(i128, i128)> = e.storage().instance().get(&key);
        stored.unwrap_or((0, 0))
    }

    fn save_user_position(e: &Env, user: &Address, reserve_index: u32, collateral: i128, liability: i128) {
        let key = (symbol_short!("pos"), user.clone(), reserve_index);
        e.storage().instance().set(&key, &(collateral, liability));
        e.storage().instance().extend_ttl(1000, 2000);
    }

    fn get_pi_price(e: &Env, oracle: &Address) -> i128 {
        e.invoke_contract::<i128>(oracle, &symbol_short!("get_price"), Vec::new(e))
    }

    /// Returns collateral_base and liability_base (oracle base units) for a user.
    fn calc_exposure_base(e: &Env, user: &Address) -> (i128, i128) {
        let config = Self::load_config(e);
        let res_list = Self::get_res_list(e);
        let pi_price = Self::get_pi_price(e, &config.oracle);

        let mut collateral_base: i128 = 0;
        let mut liability_base: i128 = 0;

        for i in 0..res_list.len() {
            let asset = res_list.get_unchecked(i);
            let rcfg = Self::get_reserve_config(e, &asset);
            let rdata = Self::get_reserve_data(e, &asset);
            let (b_bal, d_bal) = Self::load_user_position(e, user, i);

            if b_bal > 0 && rcfg.c_factor > 0 {
                let asset_value = b_bal * rdata.b_rate / SCALAR_12;
                let price = if i == 0 { pi_price } else { DECIMALS }; // Pi = index 0, PUSD = 1 USD
                collateral_base += asset_value * price * (i128::from(rcfg.c_factor)) / DECIMALS / SCALAR_7;
            }
            if d_bal > 0 && rcfg.l_factor > 0 {
                let asset_value = d_bal * rdata.d_rate / SCALAR_12;
                let price = if i == 0 { pi_price } else { DECIMALS };
                liability_base += asset_value * price * SCALAR_7 / (i128::from(rcfg.l_factor)) / DECIMALS;
            }
        }

        (collateral_base, liability_base)
    }

    /// Health check: collateral_base >= liability_base (in oracle base units).
    /// collateral_base = sum( collateral_i * price_i * c_factor_i ), liability_base = sum( liability_j * price_j / l_factor_j )
    fn check_health(e: &Env, from: &Address) -> bool {
        let (collateral_base, liability_base) = Self::calc_exposure_base(e, from);
        liability_base == 0 || collateral_base >= liability_base
    }

    /// Health factor in 7-decimal fixed point. Returns a large number when no debt.
    pub fn health_factor(e: Env, user: Address) -> i128 {
        let (collateral_base, liability_base) = Self::calc_exposure_base(&e, &user);
        if liability_base <= 0 {
            return 9_999_999_999_999i128;
        }
        collateral_base * SCALAR_7 / liability_base
    }

    pub fn is_liquidatable(e: Env, user: Address) -> bool {
        !Self::check_health(&e, &user)
    }

    /// Submit a batch of requests. from = position owner, spender = token sender, to = token receiver.
    pub fn submit(e: Env, from: Address, spender: Address, to: Address, requests: Vec<Request>) {
        from.require_auth();
        spender.require_auth();
        to.require_auth();

        let config = Self::load_config(&e);
        if config.status > 1 {
            panic!("pool frozen or restricted");
        }

        let _res_list = Self::get_res_list(&e);

        SubmitEvent {
            from: from.clone(),
            spender: spender.clone(),
            to: to.clone(),
            request_count: requests.len(),
        }
        .publish(&e);

        for i in 0..requests.len() {
            let req = requests.get_unchecked(i);
            let request_type = req.request_type;
            let asset = req.address.clone();
            let amount = req.amount;

            if amount <= 0 {
                panic!("amount must be positive");
            }

            let rcfg = Self::get_reserve_config(&e, &asset);
            if !rcfg.enabled {
                panic!("reserve disabled");
            }

            let reserve_index = rcfg.index;
            let token_client = token::Client::new(&e, &asset);
            let pool_addr = e.current_contract_address();

            RequestEvent {
                from: from.clone(),
                kind: request_type,
                asset: asset.clone(),
                amount,
            }
            .publish(&e);

            match request_type {
                0 => {
                    // Supply: spender sends tokens to pool, from gets b_tokens
                    token_client.transfer(&spender, &pool_addr, &amount);
                    let rdata = Self::get_reserve_data(&e, &asset);
                    let b_tokens = amount * SCALAR_12 / rdata.b_rate;
                    let (c, l) = Self::load_user_position(&e, &from, reserve_index);
                    Self::save_user_position(&e, &from, reserve_index, c + b_tokens, l);
                    let mut new_data = rdata;
                    new_data.b_supply += b_tokens;
                    Self::set_reserve_data(&e, &asset, &new_data);
                }
                1 => {
                    // Withdraw: amount = underlying to withdraw; from burns b_tokens, to receives tokens
                    let (c, l) = Self::load_user_position(&e, &from, reserve_index);
                    let rdata = Self::get_reserve_data(&e, &asset);
                    let b_burn = amount * SCALAR_12 / rdata.b_rate;
                    if c < b_burn {
                        panic!("insufficient supply");
                    }
                    token_client.transfer(&pool_addr, &to, &amount);
                    Self::save_user_position(&e, &from, reserve_index, c - b_burn, l);
                    let mut new_data = rdata;
                    new_data.b_supply -= b_burn;
                    Self::set_reserve_data(&e, &asset, &new_data);
                }
                2 => {
                    // SupplyCollateral: same as Supply (Pi as collateral)
                    token_client.transfer(&spender, &pool_addr, &amount);
                    let rdata = Self::get_reserve_data(&e, &asset);
                    let b_tokens = amount * SCALAR_12 / rdata.b_rate;
                    let (c, l) = Self::load_user_position(&e, &from, reserve_index);
                    Self::save_user_position(&e, &from, reserve_index, c + b_tokens, l);
                    let mut new_data = rdata;
                    new_data.b_supply += b_tokens;
                    Self::set_reserve_data(&e, &asset, &new_data);
                }
                3 => {
                    // WithdrawCollateral: amount = underlying to withdraw
                    let (c, l) = Self::load_user_position(&e, &from, reserve_index);
                    let rdata = Self::get_reserve_data(&e, &asset);
                    let b_burn = amount * SCALAR_12 / rdata.b_rate;
                    if c < b_burn {
                        panic!("insufficient collateral");
                    }
                    token_client.transfer(&pool_addr, &to, &amount);
                    Self::save_user_position(&e, &from, reserve_index, c - b_burn, l);
                    let mut new_data = rdata;
                    new_data.b_supply -= b_burn;
                    Self::set_reserve_data(&e, &asset, &new_data);
                }
                4 => {
                    // Borrow: from gets d_tokens, to receives tokens from pool
                    let rdata = Self::get_reserve_data(&e, &asset);
                    let d_tokens = amount * SCALAR_12 / rdata.d_rate;
                    let (c, l) = Self::load_user_position(&e, &from, reserve_index);
                    Self::save_user_position(&e, &from, reserve_index, c, l + d_tokens);
                    let mut new_data = rdata;
                    new_data.d_supply += d_tokens;
                    Self::set_reserve_data(&e, &asset, &new_data);
                    token_client.transfer(&pool_addr, &to, &amount);
                }
                5 => {
                    // Repay: spender sends tokens to pool, from's d_tokens decrease
                    token_client.transfer(&spender, &pool_addr, &amount);
                    let rdata = Self::get_reserve_data(&e, &asset);
                    let (c, l) = Self::load_user_position(&e, &from, reserve_index);
                    let d_burn = (amount * SCALAR_12 / rdata.d_rate).min(l);
                    Self::save_user_position(&e, &from, reserve_index, c, l - d_burn);
                    let mut new_data = rdata;
                    new_data.d_supply -= d_burn;
                    Self::set_reserve_data(&e, &asset, &new_data);
                }
                _ => panic!("bad request type"),
            }
        }

        if !Self::check_health(&e, &from) {
            panic!("health factor would be below 1");
        }
    }

    pub fn get_config(e: Env) -> PoolConfig {
        Self::load_config(&e)
    }

    pub fn get_reserve_list(e: Env) -> Vec<Address> {
        Self::get_res_list(&e)
    }

    pub fn get_reserve(e: Env, asset: Address) -> (ReserveConfig, ReserveData) {
        let cfg = Self::get_reserve_config(&e, &asset);
        let data = Self::get_reserve_data(&e, &asset);
        (cfg, data)
    }

    /// Accrue interest for a reserve (advances b_rate/d_rate). Callable by anyone;
    /// safe to call repeatedly. 4.1.
    pub fn accrue_interest(e: Env, asset: Address) {
        Self::accrue_reserve(&e, &asset);
    }

    pub fn get_positions(e: Env, address: Address) -> Positions {
        let res_list = Self::get_res_list(&e);
        let mut collateral = Map::new(&e);
        let mut liabilities = Map::new(&e);
        for i in 0..res_list.len() {
            let (c, l) = Self::load_user_position(&e, &address, i);
            if c > 0 {
                collateral.set(i, c);
            }
            if l > 0 {
                liabilities.set(i, l);
            }
        }
        Positions { collateral, liabilities }
    }

    /// Pause/unpause (admin only)
    pub fn set_paused(e: Env, admin: Address, paused: bool) {
        admin.require_auth();
        let mut config = Self::load_config(&e);
        if admin != config.admin {
            panic!("not admin");
        }
        config.status = if paused { 2 } else { 1 };
        e.storage().instance().set(&symbol_short!("config"), &config);
    }

    /// Liquidate undercollateralized position. Liquidator repays PUSD (reserve index 1), receives Pi (index 0) + 5% bonus.
    pub fn liquidate(e: Env, liquidator: Address, user: Address, repay_amount: i128) {
        liquidator.require_auth();
        if repay_amount <= 0 {
            panic!("repay amount must be positive");
        }
        if Self::check_health(&e, &user) {
            panic!("position not liquidatable");
        }
        let res_list = Self::get_res_list(&e);
        let pi_token = res_list.get_unchecked(0);
        let pusd_token = res_list.get_unchecked(1);
        let pi_price = Self::get_pi_price(&e, &Self::load_config(&e).oracle);

        let pusd_data = Self::get_reserve_data(&e, &pusd_token);
        let (_, user_pusd_liability) = Self::load_user_position(&e, &user, 1);
        let repay_d = (repay_amount * SCALAR_12 / pusd_data.d_rate).min(user_pusd_liability);
        let repay_actual = repay_d * pusd_data.d_rate / SCALAR_12;

        let pi_seize_underlying = repay_actual * 105 * DECIMALS / (100 * pi_price); // 5% bonus
        let pi_data = Self::get_reserve_data(&e, &pi_token);
        let pi_b_tokens_seize = pi_seize_underlying * SCALAR_12 / pi_data.b_rate;

        let (user_pi_c, user_pi_l) = Self::load_user_position(&e, &user, 0);
        if user_pi_c < pi_b_tokens_seize {
            panic!("insufficient collateral to seize");
        }

        token::Client::new(&e, &pusd_token).transfer(&liquidator, &e.current_contract_address(), &repay_actual);
        token::Client::new(&e, &pi_token).transfer(&e.current_contract_address(), &liquidator, &pi_seize_underlying);

        Self::save_user_position(&e, &user, 0, user_pi_c - pi_b_tokens_seize, user_pi_l);
        Self::save_user_position(&e, &user, 1, 0, user_pusd_liability - repay_d);

        let mut new_pi = pi_data;
        new_pi.b_supply -= pi_b_tokens_seize;
        Self::set_reserve_data(&e, &pi_token, &new_pi);
        let mut new_pusd = pusd_data;
        new_pusd.d_supply -= repay_d;
        Self::set_reserve_data(&e, &pusd_token, &new_pusd);

        LiquidateEvent {
            user: user.clone(),
            liquidator: liquidator.clone(),
            repay_underlying: repay_actual,
            seize_underlying: pi_seize_underlying,
        }
        .publish(&e);
    }
}

#[cfg(test)]
mod test;
