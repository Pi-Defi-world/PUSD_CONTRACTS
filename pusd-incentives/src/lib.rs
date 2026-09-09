#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, symbol_short, Address, Env,
    Map, Vec,
};

// Minimal incentives registry:
// - Admin can grant/revoke rewards for a user.
// - Rewards encode fee discount (bps) + liquidation immunity flag + optional expiry ledger.
// - Backend can read rewards to compute effective fee discounts without DB state.

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Paused,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reward {
    pub reward_type: u32,       // app-defined enum
    pub fee_discount_bps: u32,  // 0..200 for max 2% at 10_000 bps scale on backend
    pub liquidation_immunity: bool,
    pub expires_ledger: u32,    // 0 = no expiry
    pub active: bool,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    NotAdmin = 1,
    Paused = 2,
}

#[contractevent(topics = ["RewardSet"])]
pub struct RewardSetEvent {
    #[topic]
    pub user: Address,
    pub reward_type: u32,
    pub active: bool,
}

fn read_admin(e: &Env) -> Address {
    e.storage()
        .instance()
        .get::<DataKey, Address>(&DataKey::Admin)
        .unwrap()
}

fn paused(e: &Env) -> bool {
    e.storage()
        .instance()
        .get::<DataKey, bool>(&DataKey::Paused)
        .unwrap_or(false)
}

fn key(user: &Address) -> (soroban_sdk::Symbol, Address) {
    (symbol_short!("rew"), user.clone())
}

#[contract]
pub struct PusdIncentives;

#[contractimpl]
impl PusdIncentives {
    pub fn initialize(e: Env, admin: Address) {
        if e.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        admin.require_auth();
        e.storage().instance().set(&DataKey::Admin, &admin);
        e.storage().instance().set(&DataKey::Paused, &false);
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

    pub fn set_reward(
        e: Env,
        admin: Address,
        user: Address,
        reward: Reward,
    ) -> Result<(), Error> {
        let a = read_admin(&e);
        if admin != a {
            return Err(Error::NotAdmin);
        }
        admin.require_auth();
        if paused(&e) {
            return Err(Error::Paused);
        }
        let mut m: Map<u32, Reward> = e
            .storage()
            .persistent()
            .get(&key(&user))
            .unwrap_or(Map::new(&e));
        m.set(reward.reward_type, reward.clone());
        e.storage().persistent().set(&key(&user), &m);
        e.storage()
            .persistent()
            .extend_ttl(&key(&user), 1000, 2000);
        RewardSetEvent {
            user,
            reward_type: reward.reward_type,
            active: reward.active,
        }
        .publish(&e);
        Ok(())
    }

    pub fn get_rewards(e: Env, user: Address) -> Vec<Reward> {
        let now = e.ledger().sequence();
        let m: Map<u32, Reward> = e
            .storage()
            .persistent()
            .get(&key(&user))
            .unwrap_or(Map::new(&e));
        let mut out = Vec::new(&e);
        for (k, r) in m.iter() {
            let _ = k;
            if !r.active {
                continue;
            }
            if r.expires_ledger != 0 && now >= r.expires_ledger {
                continue;
            }
            out.push_back(r);
        }
        out
    }
}

