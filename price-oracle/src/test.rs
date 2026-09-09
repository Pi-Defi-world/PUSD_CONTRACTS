#![cfg(test)]
use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

#[test]
fn test_initialize_and_set_price() {
    let e = soroban_sdk::Env::default();
    let admin = Address::generate(&e);

    let oracle = PriceOracle::new(&e);
    oracle.initialize(&admin);

    e.mock_all_auths();
    oracle.set_price(&admin, &2_500_000); // 0.25 USD
    assert_eq!(oracle.get_price(), 2_500_000);
}
