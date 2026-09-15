#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype,
    Address, Env, Symbol, Vec,
    token::TokenClient,
};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Config,
    Payroll(u64),
    EmployeeClaim(u64, Address),
}

#[contracttype]
#[derive(Clone)]
pub struct PayrollConfig {
    pub admin: Address,
    pub pusd_token: Address,
    pub paused: bool,
    pub next_id: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct PayrollAgreement {
    pub id: u64,
    pub employer: Address,
    pub employee: Address,
    pub amount_per_period: i128,
    pub period_ledgers: u32,
    pub total_periods: u32,
    pub start_ledger: u32,
    pub total_amount: i128,
    pub claimed_amount: i128,
    pub status: u32, // 0=ACTIVE, 1=COMPLETED, 2=CANCELLED
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PayrollError {
    NotAdmin = 1,
    Paused = 2,
    AlreadyInitialized = 3,
    AgreementNotFound = 4,
    NotEmployer = 5,
    NotEmployee = 6,
    PeriodNotVested = 7,
    InsufficientFunds = 8,
    AlreadyClaimed = 9,
    AgreementInactive = 10,
    TooManyEmployees = 11,
    InvalidAmount = 12,
}

fn get_config(env: &Env) -> PayrollConfig {
    env.storage().instance().get(&DataKey::Config).unwrap()
}

fn set_config(env: &Env, config: &PayrollConfig) {
    env.storage().instance().set(&DataKey::Config, config);
}

fn get_payroll(env: &Env, id: u64) -> Option<PayrollAgreement> {
    env.storage().persistent().get(&DataKey::Payroll(id))
}

fn set_payroll(env: &Env, payroll: &PayrollAgreement) {
    let key = DataKey::Payroll(payroll.id);
    env.storage().persistent().set(&key, payroll);
}

fn is_claimed(env: &Env, payroll_id: u64, employee: &Address) -> bool {
    env.storage().persistent()
        .get::<DataKey, bool>(&DataKey::EmployeeClaim(payroll_id, employee.clone()))
        .unwrap_or(false)
}

fn mark_claimed(env: &Env, payroll_id: u64, employee: &Address) {
    env.storage().persistent()
        .set(&DataKey::EmployeeClaim(payroll_id, employee.clone()), &true);
}

#[contract]
pub struct PusdPayroll;

#[contractimpl]
impl PusdPayroll {
    pub fn initialize(env: Env, admin: Address, pusd_token: Address) -> Result<(), PayrollError> {
        if env.storage().instance().has(&DataKey::Config) {
            return Err(PayrollError::AlreadyInitialized);
        }
        admin.require_auth();

        let config = PayrollConfig {
            admin,
            pusd_token,
            paused: false,
            next_id: 1,
        };
        set_config(&env, &config);
        Ok(())
    }

    pub fn create_payroll(
        env: Env,
        employer: Address,
        employees: Vec<Address>,
        amounts_per_period: Vec<i128>,
        period_ledgers: u32,
        total_periods: u32,
        start_ledger: u32,
    ) -> Result<Vec<u64>, PayrollError> {
        let config = get_config(&env);
        if config.paused {
            return Err(PayrollError::Paused);
        }
        employer.require_auth();

        if employees.len() == 0 || employees.len() > 200 {
            return Err(PayrollError::TooManyEmployees);
        }
        if employees.len() != amounts_per_period.len() {
            return Err(PayrollError::TooManyEmployees);
        }

        let token_client = TokenClient::new(&env, &config.pusd_token);

        let mut total_commitment: i128 = 0;
        for i in 0..employees.len() {
            let amount_per_period = amounts_per_period.get(i).unwrap();
            if amount_per_period <= 0 {
                return Err(PayrollError::InvalidAmount);
            }
            total_commitment += amount_per_period * (total_periods as i128);
        }

        token_client.transfer(&employer, &env.current_contract_address(), &total_commitment);

        let mut next_id = config.next_id;
        let mut created_ids = Vec::new(&env);

        for i in 0..employees.len() {
            let employee = employees.get(i).unwrap();
            let amount_per_period = amounts_per_period.get(i).unwrap();
            let total_amount = amount_per_period * (total_periods as i128);

            let agreement = PayrollAgreement {
                id: next_id,
                employer: employer.clone(),
                employee: employee.clone(),
                amount_per_period,
                period_ledgers,
                total_periods,
                start_ledger,
                total_amount,
                claimed_amount: 0,
                status: 0,
            };

            set_payroll(&env, &agreement);
            created_ids.push_back(next_id);
            next_id += 1;

            env.events().publish(
                (Symbol::new(&env, "pay_create"), employer.clone(), employee.clone()),
                total_amount,
            );
        }

        let mut new_config = config;
        new_config.next_id = next_id;
        set_config(&env, &new_config);

        Ok(created_ids)
    }

    pub fn claim_salary(
        env: Env,
        employee: Address,
        payroll_id: u64,
    ) -> Result<i128, PayrollError> {
        let config = get_config(&env);
        if config.paused {
            return Err(PayrollError::Paused);
        }
        employee.require_auth();

        let mut agreement = get_payroll(&env, payroll_id)
            .ok_or(PayrollError::AgreementNotFound)?;

        if agreement.employee != employee {
            return Err(PayrollError::NotEmployee);
        }
        if agreement.status != 0 {
            return Err(PayrollError::AgreementInactive);
        }

        // 4.2: derive the ledger from the protocol, never from caller input.
        let current_ledger = env.ledger().sequence();
        let elapsed = current_ledger.saturating_sub(agreement.start_ledger);
        let completed_periods = (elapsed / agreement.period_ledgers).min(agreement.total_periods);
        let unclaimed_periods = completed_periods.saturating_sub(
            (agreement.claimed_amount / agreement.amount_per_period) as u32
        );

        if unclaimed_periods == 0 {
            return Err(PayrollError::PeriodNotVested);
        }

        let claim_amount = (unclaimed_periods as i128) * agreement.amount_per_period;
        let token_client = TokenClient::new(&env, &config.pusd_token);

        token_client.transfer(
            &env.current_contract_address(),
            &employee,
            &claim_amount,
        );

        agreement.claimed_amount += claim_amount;

        if agreement.claimed_amount >= agreement.total_amount {
            agreement.status = 1;
        }

        mark_claimed(&env, payroll_id, &employee);
        set_payroll(&env, &agreement);

        env.events().publish(
            (Symbol::new(&env, "sal_claim"), payroll_id, employee),
            claim_amount,
        );

        Ok(claim_amount)
    }

    pub fn cancel_payroll(
        env: Env,
        employer: Address,
        payroll_id: u64,
    ) -> Result<i128, PayrollError> {
        let config = get_config(&env);
        employer.require_auth();

        let mut agreement = get_payroll(&env, payroll_id)
            .ok_or(PayrollError::AgreementNotFound)?;

        if agreement.employer != employer {
            return Err(PayrollError::NotEmployer);
        }
        if agreement.status != 0 {
            return Err(PayrollError::AgreementInactive);
        }

        agreement.status = 2;
        let unvested = agreement.total_amount - agreement.claimed_amount;

        if unvested > 0 {
            let token_client = TokenClient::new(&env, &config.pusd_token);
            token_client.transfer(
                &env.current_contract_address(),
                &employer,
                &unvested,
            );
        }

        set_payroll(&env, &agreement);

        env.events().publish(
            (Symbol::new(&env, "pay_cancel"), payroll_id, employer),
            unvested,
        );

        Ok(unvested)
    }

    pub fn get_agreement(env: Env, payroll_id: u64) -> Result<PayrollAgreement, PayrollError> {
        get_payroll(&env, payroll_id).ok_or(PayrollError::AgreementNotFound)
    }

    pub fn get_vested_amount(
        env: Env,
        payroll_id: u64,
    ) -> Result<i128, PayrollError> {
        let agreement = get_payroll(&env, payroll_id)
            .ok_or(PayrollError::AgreementNotFound)?;

        // 4.2: derive the ledger from the protocol, never from caller input.
        let current_ledger = env.ledger().sequence();
        let elapsed = current_ledger.saturating_sub(agreement.start_ledger);
        let completed_periods = (elapsed / agreement.period_ledgers).min(agreement.total_periods);
        let vested = (completed_periods as i128) * agreement.amount_per_period;

        Ok(vested)
    }

    pub fn admin(env: Env) -> Address {
        get_config(&env).admin
    }

    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), PayrollError> {
        let mut config = get_config(&env);
        config.admin.require_auth();
        config.admin = new_admin;
        set_config(&env, &config);
        Ok(())
    }

    pub fn set_paused(env: Env, paused: bool) -> Result<(), PayrollError> {
        let mut config = get_config(&env);
        config.admin.require_auth();
        config.paused = paused;
        set_config(&env, &config);
        Ok(())
    }
}
