#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env,
};
use soroban_sdk::token;

// Simple PUSD streaming payments:
// - Payer funds a stream to a payee.
// - Linear release per ledger between start_ledger and end_ledger.
// - Payee can periodically withdraw accrued amount.

#[contracttype]
#[derive(Clone)]
pub struct Stream {
    pub payer: Address,
    pub payee: Address,
    pub total_amount: i128,
    pub withdrawn_amount: i128,
    pub pusd_token: Address,
    pub start_ledger: u32,
    pub end_ledger: u32,
    pub cancelled: bool,
}

#[contracttype]
pub enum DataKey {
    Stream(Address),
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
    InvalidSchedule = 5,
    StreamNotFound = 6,
}

use soroban_sdk::panic_with_error;

fn require(e: &Env, cond: bool, err: Error) {
    if !cond {
        panic_with_error!(e, err);
    }
}

#[contract]
pub struct PusdStreaming;

#[contractimpl]
impl PusdStreaming {
    pub fn initialize(e: Env, admin: Address, pusd_token: Address) {
        if e.storage().instance().has(&DataKey::Admin) {
            return;
        }
        admin.require_auth();
        e.storage().instance().set(&DataKey::Admin, &admin);
        e.storage().instance().set(&DataKey::PusdToken, &pusd_token);
    }

    pub fn create_stream(
        e: Env,
        stream_id: Address,
        payer: Address,
        payee: Address,
        total_amount: i128,
        start_ledger: u32,
        end_ledger: u32,
    ) {
        payer.require_auth();
        require(&e, total_amount > 0, Error::InvalidAmount);
        require(&e, end_ledger > start_ledger, Error::InvalidSchedule);

        let pusd_token: Address = e
            .storage()
            .instance()
            .get(&DataKey::PusdToken)
            .unwrap();
        let client = token::Client::new(&e, &pusd_token);

        // Pull full amount up front from payer.
        client.transfer(&payer, &e.current_contract_address(), &total_amount);

        let stream = Stream {
            payer,
            payee,
            total_amount,
            withdrawn_amount: 0,
            pusd_token,
            start_ledger,
            end_ledger,
            cancelled: false,
        };

        e.storage().instance().set(&DataKey::Stream(stream_id), &stream);
    }

    fn accrued_amount(e: &Env, s: &Stream) -> i128 {
        let ledger = e.ledger().sequence();
        if ledger <= s.start_ledger {
            0
        } else if ledger >= s.end_ledger {
            s.total_amount
        } else {
            let elapsed = (ledger - s.start_ledger) as i128;
            let total = (s.end_ledger - s.start_ledger) as i128;
            s.total_amount * elapsed / total
        }
    }

    pub fn withdraw(e: Env, stream_id: Address, payee: Address) {
        payee.require_auth();
        let key = DataKey::Stream(stream_id.clone());
        let mut stream: Stream = e.storage().instance().get(&key).unwrap_or_else(|| {
            panic_with_error!(&e, Error::StreamNotFound)
        });
        require(&e, !stream.cancelled, Error::StreamNotFound);
        require(&e, payee == stream.payee, Error::NotPayee);

        let accrued = Self::accrued_amount(&e, &stream);
        let owed = accrued - stream.withdrawn_amount;
        if owed <= 0 {
            return;
        }

        let client = token::Client::new(&e, &stream.pusd_token);
        client.transfer(
            &e.current_contract_address(),
            &stream.payee,
            &owed,
        );

        stream.withdrawn_amount += owed;
        e.storage().instance().set(&key, &stream);
    }

    pub fn cancel(e: Env, stream_id: Address, payer: Address) {
        payer.require_auth();
        let key = DataKey::Stream(stream_id.clone());
        let mut stream: Stream = e.storage().instance().get(&key).unwrap_or_else(|| {
            panic_with_error!(&e, Error::StreamNotFound)
        });
        require(&e, payer == stream.payer, Error::NotPayer);
        if stream.cancelled {
            return;
        }

        let accrued = Self::accrued_amount(&e, &stream);
        let remaining = stream.total_amount - accrued;
        let client = token::Client::new(&e, &stream.pusd_token);

        if remaining > 0 {
            client.transfer(
                &e.current_contract_address(),
                &stream.payer,
                &remaining,
            );
        }

        stream.cancelled = true;
        e.storage().instance().set(&key, &stream);
    }

    pub fn get_stream(e: Env, stream_id: Address) -> Option<Stream> {
        e.storage().instance().get(&DataKey::Stream(stream_id))
    }
}

