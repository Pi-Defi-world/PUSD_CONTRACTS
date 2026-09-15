#![cfg(test)]
use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token::{StellarAssetClient, Token},
    vec, Address, Env,
};
use super::PusdPoolClient;

#[contract]
pub struct MockOracle;

#[contractimpl]
impl MockOracle {
    pub fn get_price(_e: Env) -> i128 {
        DECIMALS // 1 USD per Pi (7-decimal base)
    }
    pub fn set_price(_e: Env, _p: i128) {}
}

fn setup() -> (Env, Address, Address, Address, Address, Address) {
    let e = Env::default();
    e.mock_all_auths();
    let admin = Address::generate(&e);
    let oracle = e.register(MockOracle, ());
    let pi_asset = e.register_stellar_asset_contract(admin.clone());
    let pusd_asset = e.register_stellar_asset_contract(admin.clone());
    let pi = pi_asset.address();
    let pusd = pusd_asset.address();

    let pool = e.register(PusdPool, ());
    PusdPoolClient::new(&e, &pool).initialize(
        &admin,
        &oracle,
        &pi,
        &pusd,
        &0,
        &10,
        &0,
    );

    (e, pool, pi, pusd, admin, oracle)
}

fn mint(e: &Env, token: &Token, to: &Address, amount: i128) {
    StellarAssetClient::new(e, &token.address()).mint(to, &amount);
}

#[test]
fn test_accrual_increases_d_rate() {
    let (e, pool, pi, pusd, _admin, _oracle) = setup();
    let user = Address::generate(&e);
    let pi_tok = Token::new(&e, &pi);
    let pusd_tok = Token::new(&e, &pusd);
    mint(&e, &pi_tok, &user, 10 * SCALAR_7);
    mint(&e, &pusd_tok, &user, 100 * SCALAR_7);

    let client = PusdPoolClient::new(&e, &pool);
    // Supply 10 Pi as collateral, borrow 5 PUSD.
    client.submit(
        &user,
        &user,
        &user,
        &vec![&e; Request {
            request_type: 2,
            address: pi.clone(),
            amount: 10 * SCALAR_7,
        }, Request {
            request_type: 4,
            address: pusd.clone(),
            amount: 5 * SCALAR_7,
        }],
    );

    let before = client.get_reserve(&pusd).1.d_rate;
    assert_eq!(before, SCALAR_12);

    // Advance one year and accrue.
    e.ledger().set_timestamp(e.ledger().timestamp() + 31_536_000);
    client.accrue_interest(&pusd);

    let after = client.get_reserve(&pusd).1.d_rate;
    assert!(after > SCALAR_12, "d_rate should accrue upward");
}

#[test]
fn test_health_factor_and_liquidation() {
    let (e, pool, pi, pusd, _admin, _oracle) = setup();
    let user = Address::generate(&e);
    let liquidator = Address::generate(&e);
    let pi_tok = Token::new(&e, &pi);
    let pusd_tok = Token::new(&e, &pusd);
    mint(&e, &pi_tok, &user, 10 * SCALAR_7);
    mint(&e, &pusd_tok, &user, 100 * SCALAR_7);
    mint(&e, &pusd_tok, &liquidator, 100 * SCALAR_7);

    let client = PusdPoolClient::new(&e, &pool);
    // Supply 10 Pi, borrow 9 PUSD -> unhealthy (liability > collateral).
    client.submit(
        &user,
        &user,
        &user,
        &vec![&e; Request {
            request_type: 2,
            address: pi.clone(),
            amount: 10 * SCALAR_7,
        }, Request {
            request_type: 4,
            address: pusd.clone(),
            amount: 9 * SCALAR_7,
        }],
    );

    assert!(client.health_factor(&user) < SCALAR_7, "should be unhealthy");
    assert!(client.is_liquidatable(&user));

    let pi_before = pi_tok.balance(&user);
    client.liquidate(&liquidator, &user, &(9 * SCALAR_7));
    let pi_after = pi_tok.balance(&user);
    assert!(pi_after < pi_before, "collateral should be seized");
}

#[test]
fn test_healthy_position_not_liquidatable() {
    let (e, pool, pi, pusd, _admin, _oracle) = setup();
    let user = Address::generate(&e);
    let pi_tok = Token::new(&e, &pi);
    let pusd_tok = Token::new(&e, &pusd);
    mint(&e, &pi_tok, &user, 10 * SCALAR_7);
    mint(&e, &pusd_tok, &user, 100 * SCALAR_7);

    let client = PusdPoolClient::new(&e, &pool);
    client.submit(
        &user,
        &user,
        &user,
        &vec![&e; Request {
            request_type: 2,
            address: pi.clone(),
            amount: 10 * SCALAR_7,
        }, Request {
            request_type: 4,
            address: pusd.clone(),
            amount: 3 * SCALAR_7,
        }],
    );
    assert!(client.health_factor(&user) >= SCALAR_7);
    assert!(!client.is_liquidatable(&user));
}
