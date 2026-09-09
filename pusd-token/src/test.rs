 #![cfg(test)]

 use super::*;
 use soroban_sdk::testutils::Address as _;
 use soroban_sdk::Env;

 #[test]
 fn initialize_and_metadata() {
     let env = Env::default();
     let admin = soroban_sdk::Address::generate(&env);

     PusdToken::initialize(env.clone(), admin.clone());

     assert_eq!(PusdToken::admin(env.clone()), admin);
     assert_eq!(PusdToken::decimals(env.clone()), DECIMALS);
     assert_eq!(PusdToken::total_supply(env.clone()), 0);
 }

 #[test]
 fn mint_and_transfer() {
     let env = Env::default();
     let admin = soroban_sdk::Address::generate(&env);
     let user1 = soroban_sdk::Address::generate(&env);
     let user2 = soroban_sdk::Address::generate(&env);

     PusdToken::initialize(env.clone(), admin.clone());

     // Mint to user1
     PusdToken::mint(env.clone(), admin.clone(), user1.clone(), 1_000_000).unwrap();
     assert_eq!(PusdToken::balance(env.clone(), user1.clone()), 1_000_000);
     assert_eq!(PusdToken::total_supply(env.clone()), 1_000_000);

     // Transfer from user1 to user2
     PusdToken::transfer(env.clone(), user1.clone(), user2.clone(), 400_000).unwrap();
     assert_eq!(PusdToken::balance(env.clone(), user1.clone()), 600_000);
     assert_eq!(PusdToken::balance(env.clone(), user2.clone()), 400_000);
 }

 #[test]
 fn approve_and_transfer_from() {
     let env = Env::default();
     let admin = soroban_sdk::Address::generate(&env);
     let owner = soroban_sdk::Address::generate(&env);
     let spender = soroban_sdk::Address::generate(&env);
     let recipient = soroban_sdk::Address::generate(&env);

     PusdToken::initialize(env.clone(), admin.clone());
     PusdToken::mint(env.clone(), admin.clone(), owner.clone(), 500_000).unwrap();

     PusdToken::approve(env.clone(), owner.clone(), spender.clone(), 200_000).unwrap();
     assert_eq!(
         PusdToken::allowance(env.clone(), owner.clone(), spender.clone()),
         200_000
     );

     PusdToken::transfer_from(
         env.clone(),
         spender.clone(),
         owner.clone(),
         recipient.clone(),
         150_000,
     )
     .unwrap();

     assert_eq!(PusdToken::balance(env.clone(), owner.clone()), 350_000);
     assert_eq!(PusdToken::balance(env.clone(), recipient.clone()), 150_000);
     assert_eq!(
         PusdToken::allowance(env.clone(), owner.clone(), spender.clone()),
         50_000
     );
 }

 #[test]
 fn pause_blocks_transfers() {
     let env = Env::default();
     let admin = soroban_sdk::Address::generate(&env);
     let user1 = soroban_sdk::Address::generate(&env);
     let user2 = soroban_sdk::Address::generate(&env);

     PusdToken::initialize(env.clone(), admin.clone());
     PusdToken::mint(env.clone(), admin.clone(), user1.clone(), 100_000).unwrap();

     // Pause the contract
     PusdToken::set_paused(env.clone(), admin.clone(), true).unwrap();

     let res = PusdToken::transfer(env.clone(), user1.clone(), user2.clone(), 10_000);
     assert!(matches!(res, Err(Error::Paused)));
 }

