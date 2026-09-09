#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env,
};
use soroban_sdk::token;

// Simple PUSD-based escrow contract:
// - Payer funds an escrow for a payee in PUSD.
// - Funds are held until either released by payer or refunded by timeout.
// - PUSD token contract address is configured at initialization.

#[contracttype]
#[derive(Clone)]
pub struct Escrow {
    pub payer: Address,
    pub payee: Address,
    pub amount: i128,
    pub pusd_token: Address,
    pub deadline_ledger: u32,
    pub released: bool,
    pub refunded: bool,
}

#[contracttype]
pub enum DataKey {
    Escrow(Address),
    Admin,
    PusdToken,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    NotAdmin = 1,
    NotPayer = 2,
    NotPayee = 3,
    InvalidAmount = 4,
    EscrowNotFound = 5,
    EscrowAlreadyCompleted = 6,
    BeforeDeadline = 7,
}

pub trait PusdTokenClient {
    fn transfer(e: &Env, from: &Address, to: &Address, amount: i128);
}

fn require(e: &Env, cond: bool, err: Error) {
    if !cond {
        panic_with_error!(e, err);
    }
}

use soroban_sdk::panic_with_error;

#[contract]
pub struct PusdEscrow;

#[contractimpl]
impl PusdEscrow {
    pub fn initialize(e: Env, admin: Address, pusd_token: Address) {
        if e.storage().instance().has(&DataKey::Admin) {
            return;
        }
        admin.require_auth();
        e.storage().instance().set(&DataKey::Admin, &admin);
        e.storage().instance().set(&DataKey::PusdToken, &pusd_token);
    }

    pub fn create_escrow(
        e: Env,
        payer: Address,
        payee: Address,
        amount: i128,
        deadline_ledger: u32,
    ) {
        payer.require_auth();
        require(&e, amount > 0, Error::InvalidAmount);

        let pusd_token: Address = e
            .storage()
            .instance()
            .get(&DataKey::PusdToken)
            .unwrap();

        // Pull PUSD from payer into this contract.
        let client = token::Client::new(&e, &pusd_token);
        client.transfer(&payer, &e.current_contract_address(), &amount);

        let escrow = Escrow {
            payer: payer.clone(),
            payee,
            amount,
            pusd_token,
            deadline_ledger,
            released: false,
            refunded: false,
        };

        e.storage().instance().set(&DataKey::Escrow(payer), &escrow);
    }

    pub fn release(e: Env, payer: Address) {
        payer.require_auth();
        let key = DataKey::Escrow(payer.clone());
        let mut escrow: Escrow = e.storage().instance().get(&key).unwrap_or_else(|| {
            panic_with_error!(&e, Error::EscrowNotFound)
        });

        require(&e, !escrow.released && !escrow.refunded, Error::EscrowAlreadyCompleted);

        let client = token::Client::new(&e, &escrow.pusd_token);
        client.transfer(
            &e.current_contract_address(),
            &escrow.payee,
            &escrow.amount,
        );

        escrow.released = true;
        e.storage().instance().set(&key, &escrow);
    }

    pub fn refund(e: Env, payer: Address) {
        payer.require_auth();
        let key = DataKey::Escrow(payer.clone());
        let mut escrow: Escrow = e.storage().instance().get(&key).unwrap_or_else(|| {
            panic_with_error!(&e, Error::EscrowNotFound)
        });

        require(&e, !escrow.released && !escrow.refunded, Error::EscrowAlreadyCompleted);
        require(
            &e,
            e.ledger().sequence() >= escrow.deadline_ledger,
            Error::BeforeDeadline,
        );

        let client = token::Client::new(&e, &escrow.pusd_token);
        client.transfer(
            &e.current_contract_address(),
            &escrow.payer,
            &escrow.amount,
        );

        escrow.refunded = true;
        e.storage().instance().set(&key, &escrow);
    }

    pub fn get_escrow(e: Env, payer: Address) -> Option<Escrow> {
        e.storage().instance().get(&DataKey::Escrow(payer))
    }
}

