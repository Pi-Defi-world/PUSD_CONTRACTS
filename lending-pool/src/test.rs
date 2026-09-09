#![cfg(test)]
use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

#[test]
fn test_config_and_position_types() {
    let e = soroban_sdk::Env::default();
    let admin = Address::generate(&e);
    let pi_token = Address::generate(&e);
    let pusd_token = Address::generate(&e);
    let oracle = Address::generate(&e);

    let pool = LendingPool::new(&e);
    pool.initialize(
        &admin,
        &pi_token,
        &pusd_token,
        &oracle,
        8000,  // 80% max LTV
        8500,  // 85% liq threshold
        500,   // 5% liq bonus
    );

    let config = pool.config();
    assert_eq!(config.admin, admin);
    assert_eq!(config.max_ltv_bps, 8000);
    assert_eq!(config.paused, false);

    let user = Address::generate(&e);
    let pos = pool.get_position(&user);
    assert_eq!(pos.collateral, 0);
    assert_eq!(pos.debt, 0);
}
