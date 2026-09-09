#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env,
};
use soroban_sdk::token;

// Simple pull-based subscription payments:
// - Subscriber grants an allowance-like recurring limit to a merchant.
// - Merchant can pull up to `amount_per_period` each period.
// - Subscriber can cancel at any time.

#[contracttype]
#[derive(Clone)]
pub struct Subscription {
    pub subscriber: Address,
    pub merchant: Address,
    pub pusd_token: Address,
    pub amount_per_period: i128,
    pub period_ledgers: u32,
    pub start_ledger: u32,
    pub last_charged_ledger: u32,
    pub active: bool,
}

#[contracttype]
pub enum DataKey {
    Subscription(Address),
    Admin,
    PusdToken,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    NotAdmin = 1,
    NotSubscriber = 2,
    NotMerchant = 3,
    InvalidAmount = 4,
    InvalidPeriod = 5,
    SubscriptionNotFound = 6,
    Inactive = 7,
}

use soroban_sdk::panic_with_error;

fn require(e: &Env, cond: bool, err: Error) {
    if !cond {
        panic_with_error!(e, err);
    }
}

#[contract]
pub struct PusdSubscriptions;

#[contractimpl]
impl PusdSubscriptions {
    pub fn initialize(e: Env, admin: Address, pusd_token: Address) {
        if e.storage().instance().has(&DataKey::Admin) {
            return;
        }
        admin.require_auth();
        e.storage().instance().set(&DataKey::Admin, &admin);
        e.storage().instance().set(&DataKey::PusdToken, &pusd_token);
    }

    pub fn create_subscription(
        e: Env,
        id: Address,
        subscriber: Address,
        merchant: Address,
        amount_per_period: i128,
        period_ledgers: u32,
    ) {
        subscriber.require_auth();
        require(&e, amount_per_period > 0, Error::InvalidAmount);
        require(&e, period_ledgers > 0, Error::InvalidPeriod);

        let pusd_token: Address = e
            .storage()
            .instance()
            .get(&DataKey::PusdToken)
            .unwrap();

        let sub = Subscription {
            subscriber,
            merchant,
            pusd_token,
            amount_per_period,
            period_ledgers,
            start_ledger: e.ledger().sequence(),
            last_charged_ledger: 0,
            active: true,
        };

        e.storage().instance().set(&DataKey::Subscription(id), &sub);
    }

    pub fn cancel(e: Env, id: Address, subscriber: Address) {
        subscriber.require_auth();
        let key = DataKey::Subscription(id.clone());
        let mut sub: Subscription = e.storage().instance().get(&key).unwrap_or_else(|| {
            panic_with_error!(&e, Error::SubscriptionNotFound)
        });
        require(&e, sub.subscriber == subscriber, Error::NotSubscriber);
        sub.active = false;
        e.storage().instance().set(&key, &sub);
    }

    pub fn charge(e: Env, id: Address, merchant: Address) {
        merchant.require_auth();
        let key = DataKey::Subscription(id.clone());
        let mut sub: Subscription = e.storage().instance().get(&key).unwrap_or_else(|| {
            panic_with_error!(&e, Error::SubscriptionNotFound)
        });
        require(&e, sub.active, Error::Inactive);
        require(&e, sub.merchant == merchant, Error::NotMerchant);

        let now = e.ledger().sequence();
        let last = if sub.last_charged_ledger == 0 {
            sub.start_ledger
        } else {
            sub.last_charged_ledger
        };

        let periods_elapsed = (now - last) / sub.period_ledgers;
        if periods_elapsed == 0 {
            return;
        }

        let to_charge = sub.amount_per_period * (periods_elapsed as i128);

        let client = token::Client::new(&e, &sub.pusd_token);
        client.transfer(&sub.subscriber, &sub.merchant, &to_charge);

        sub.last_charged_ledger = now;
        e.storage().instance().set(&key, &sub);
    }

    pub fn get_subscription(e: Env, id: Address) -> Option<Subscription> {
        e.storage().instance().get(&DataKey::Subscription(id))
    }
}

