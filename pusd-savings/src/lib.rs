#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token, symbol_short,
    Address, Env, Symbol,
};

// Minimal on-chain savings vault:
// - Users deposit PUSD into this contract (token transfer).
// - Contract tracks per-user principal and accrued interest.
// - Admin can set APY (scaled), interest model, and pause.
// - Interest accrues linearly with ledger timestamp (seconds).
// - Compound interest support: interest compounds to principal on each accrual.

const SECONDS_PER_YEAR: i128 = 31_536_000; // 365 days
const APY_SCALE: i128 = 1_000_000; // 1e6 => 100% APY

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    PusdToken,
    Paused,
    ApyScaled,
    CompoundEnabled,
    User(Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserSavings {
    pub principal: i128,
    pub accrued_interest: i128,
    pub last_accrued_ts: u64,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    NotAdmin = 1,
    Paused = 2,
    InvalidAmount = 3,
    InsufficientBalance = 4,
}

#[contractevent(topics = ["Deposit"])]
pub struct DepositEvent {
    #[topic]
    pub user: Address,
    pub amount: i128,
}

#[contractevent(topics = ["Withdraw"])]
pub struct WithdrawEvent {
    #[topic]
    pub user: Address,
    pub amount: i128,
}

#[contractevent(topics = ["ApySet"])]
pub struct ApySetEvent {
    pub apy_scaled: i128,
}

fn read_admin(e: &Env) -> Address {
    e.storage()
        .instance()
        .get::<DataKey, Address>(&DataKey::Admin)
        .unwrap()
}

fn require_not_paused(e: &Env) -> Result<(), Error> {
    let paused = e
        .storage()
        .instance()
        .get::<DataKey, bool>(&DataKey::Paused)
        .unwrap_or(false);
    if paused {
        return Err(Error::Paused);
    }
    Ok(())
}

fn read_pusd_token(e: &Env) -> Address {
    e.storage()
        .instance()
        .get::<DataKey, Address>(&DataKey::PusdToken)
        .unwrap()
}

fn read_apy_scaled(e: &Env) -> i128 {
    e.storage()
        .instance()
        .get::<DataKey, i128>(&DataKey::ApyScaled)
        .unwrap_or(30_000) // default 3% = 0.03 * 1e6
}

fn read_user(e: &Env, user: &Address) -> UserSavings {
    e.storage()
        .persistent()
        .get::<(Symbol, Address), UserSavings>(&(symbol_short!("usr"), user.clone()))
        .unwrap_or(UserSavings {
            principal: 0,
            accrued_interest: 0,
            last_accrued_ts: e.ledger().timestamp(),
        })
}

fn write_user(e: &Env, user: &Address, s: &UserSavings) {
    e.storage()
        .persistent()
        .set(&(symbol_short!("usr"), user.clone()), s);
    e.storage()
        .persistent()
        .extend_ttl(&(symbol_short!("usr"), user.clone()), 1000, 2000);
}

fn read_compound_enabled(e: &Env) -> bool {
    e.storage()
        .instance()
        .get::<DataKey, bool>(&DataKey::CompoundEnabled)
        .unwrap_or(false)
}

fn accrue(e: &Env, user: &Address) -> UserSavings {
    let mut s = read_user(e, user);
    let now = e.ledger().timestamp();
    if now <= s.last_accrued_ts {
        return s;
    }
    if s.principal <= 0 {
        s.last_accrued_ts = now;
        return s;
    }
    let dt = (now - s.last_accrued_ts) as i128;
    let apy_scaled = read_apy_scaled(e);
    let compound_enabled = read_compound_enabled(e);

    // interest = principal * apy * dt / year
    let effective_principal = if compound_enabled {
        s.principal + s.accrued_interest
    } else {
        s.principal
    };
    let interest = effective_principal * apy_scaled * dt / APY_SCALE / SECONDS_PER_YEAR;
    if interest > 0 {
        s.accrued_interest += interest;
    }
    if compound_enabled && s.accrued_interest > 0 {
        s.principal += s.accrued_interest;
        s.accrued_interest = 0;
    }
    s.last_accrued_ts = now;
    s
}

#[contract]
pub struct PusdSavings;

#[contractimpl]
impl PusdSavings {
    pub fn initialize(e: Env, admin: Address, pusd_token: Address, apy_scaled: i128) {
        if e.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        admin.require_auth();
        e.storage().instance().set(&DataKey::Admin, &admin);
        e.storage().instance().set(&DataKey::PusdToken, &pusd_token);
        e.storage().instance().set(&DataKey::Paused, &false);
        e.storage().instance().set(&DataKey::ApyScaled, &apy_scaled);
        e.storage().instance().set(&DataKey::CompoundEnabled, &false);
        ApySetEvent { apy_scaled }.publish(&e);
    }

    pub fn set_paused(e: Env, admin: Address, paused: bool) -> Result<(), Error> {
        let a = read_admin(&e);
        if admin != a {
            return Err(Error::NotAdmin);
        }
        admin.require_auth();
        e.storage().instance().set(&DataKey::Paused, &paused);
        Ok(())
    }

    pub fn set_apy(e: Env, admin: Address, apy_scaled: i128) -> Result<(), Error> {
        let a = read_admin(&e);
        if admin != a {
            return Err(Error::NotAdmin);
        }
        admin.require_auth();
        e.storage().instance().set(&DataKey::ApyScaled, &apy_scaled);
        ApySetEvent { apy_scaled }.publish(&e);
        Ok(())
    }

    pub fn get_apy(e: Env) -> i128 {
        read_apy_scaled(&e)
    }

    pub fn is_compound_enabled(e: Env) -> bool {
        read_compound_enabled(&e)
    }

    pub fn set_compound_enabled(e: Env, admin: Address, enabled: bool) -> Result<(), Error> {
        let a = read_admin(&e);
        if admin != a {
            return Err(Error::NotAdmin);
        }
        admin.require_auth();
        e.storage().instance().set(&DataKey::CompoundEnabled, &enabled);
        Ok(())
    }

    pub fn get_account(e: Env, user: Address) -> UserSavings {
        let s = accrue(&e, &user);
        // do not persist on view; only on mutating calls
        s
    }

    pub fn deposit(e: Env, user: Address, amount: i128) -> Result<(), Error> {
        require_not_paused(&e)?;
        user.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let pusd_token = read_pusd_token(&e);
        let client = token::Client::new(&e, &pusd_token);
        client.transfer(&user, &e.current_contract_address(), &amount);

        let mut s = accrue(&e, &user);
        s.principal += amount;
        write_user(&e, &user, &s);
        DepositEvent { user, amount }.publish(&e);
        Ok(())
    }

    pub fn withdraw(e: Env, user: Address, amount: i128) -> Result<(), Error> {
        require_not_paused(&e)?;
        user.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut s = accrue(&e, &user);
        let available = s.principal + s.accrued_interest;
        if amount > available {
            return Err(Error::InsufficientBalance);
        }

        // pay from interest first, then principal
        let mut remaining = amount;
        if s.accrued_interest > 0 {
            let take = if remaining > s.accrued_interest {
                s.accrued_interest
            } else {
                remaining
            };
            s.accrued_interest -= take;
            remaining -= take;
        }
        if remaining > 0 {
            s.principal -= remaining;
        }

        let pusd_token = read_pusd_token(&e);
        let client = token::Client::new(&e, &pusd_token);
        client.transfer(&e.current_contract_address(), &user, &amount);

        write_user(&e, &user, &s);
        WithdrawEvent { user, amount }.publish(&e);
        Ok(())
    }
}

