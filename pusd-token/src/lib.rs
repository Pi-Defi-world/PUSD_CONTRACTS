 #![no_std]

 //! PUSD Soroban token contract
 //!
 //! Standard Soroban token interface with issuer-controlled mint and burn.
 //! On-chain, this contract only enforces balances, allowances, and issuer
 //! access control; reserve management and Pi bridge logic live entirely
 //! off-chain in the backend.

use soroban_sdk::{
     contract, contracterror, contractimpl, contracttype, Address, BytesN, Env,
 };

 /// Fixed token metadata for PUSD on Pi.
 const NAME: &str = "PUSD";
 const SYMBOL: &str = "PUSD";
 /// 7 decimals (1e7) to match Pi / existing PUSD conventions.
 pub const DECIMALS: u32 = 7;

 #[contracttype]
 #[derive(Clone)]
 pub enum DataKey {
     Admin,
     Paused,
     Balance(Address),
     Allowance(Address, Address),
     TotalSupply,
 }

 #[contracterror]
 #[derive(Copy, Clone, Debug, Eq, PartialEq)]
 pub enum Error {
     NotAdmin = 1,
     Paused = 2,
     InsufficientBalance = 3,
     InsufficientAllowance = 4,
 }

 #[contract]
 pub struct PusdToken;

 fn read_admin(env: &Env) -> Address {
     env.storage()
         .instance()
         .get::<DataKey, Address>(&DataKey::Admin)
         .unwrap()
 }

 fn write_admin(env: &Env, admin: &Address) {
     env.storage()
         .instance()
         .set(&DataKey::Admin, admin);
 }

 fn is_paused(env: &Env) -> bool {
     env.storage()
         .instance()
         .get::<DataKey, bool>(&DataKey::Paused)
         .unwrap_or(false)
 }

 fn set_paused(env: &Env, paused: bool) {
     env.storage()
         .instance()
         .set(&DataKey::Paused, &paused);
 }

 fn read_balance(env: &Env, addr: &Address) -> i128 {
     env.storage()
         .instance()
         .get::<DataKey, i128>(&DataKey::Balance(addr.clone()))
         .unwrap_or(0)
 }

 fn write_balance(env: &Env, addr: &Address, amount: i128) {
     env.storage()
         .instance()
         .set(&DataKey::Balance(addr.clone()), &amount);
 }

 fn read_allowance(env: &Env, from: &Address, spender: &Address) -> i128 {
     env.storage()
         .instance()
         .get::<DataKey, i128>(&DataKey::Allowance(from.clone(), spender.clone()))
         .unwrap_or(0)
 }

 fn write_allowance(env: &Env, from: &Address, spender: &Address, amount: i128) {
     env.storage()
         .instance()
         .set(&DataKey::Allowance(from.clone(), spender.clone()), &amount);
 }

 fn read_total_supply(env: &Env) -> i128 {
     env.storage()
         .instance()
         .get::<DataKey, i128>(&DataKey::TotalSupply)
         .unwrap_or(0)
 }

 fn write_total_supply(env: &Env, amount: i128) {
     env.storage()
         .instance()
         .set(&DataKey::TotalSupply, &amount);
 }

 #[contractimpl]
 impl PusdToken {
     /// Initialize the contract with an admin (issuer) address.
     ///
     /// This is expected to be called exactly once at deployment.
     pub fn initialize(env: Env, admin: Address) {
         if env
             .storage()
             .instance()
             .has(&DataKey::Admin)
         {
             panic!("already initialized");
         }
         admin.require_auth();
         write_admin(&env, &admin);
         set_paused(&env, false);
     }

     /// Returns the token name.
     pub fn name(_env: Env) -> BytesN<32> {
         let mut out = [0u8; 32];
         let b = NAME.as_bytes();
         let n = if b.len() > 32 { 32 } else { b.len() };
         out[..n].copy_from_slice(&b[..n]);
         BytesN::from_array(&_env, &out)
     }

     /// Returns the token symbol.
     pub fn symbol(_env: Env) -> BytesN<32> {
         let mut out = [0u8; 32];
         let b = SYMBOL.as_bytes();
         let n = if b.len() > 32 { 32 } else { b.len() };
         out[..n].copy_from_slice(&b[..n]);
         BytesN::from_array(&_env, &out)
     }

     /// Returns the token decimals.
     pub fn decimals(_env: Env) -> u32 {
         DECIMALS
     }

     /// Returns the total supply.
     pub fn total_supply(env: Env) -> i128 {
         read_total_supply(&env)
     }

     /// Returns the balance of `owner`.
     pub fn balance(env: Env, owner: Address) -> i128 {
         read_balance(&env, &owner)
     }

     /// Returns the allowance from `owner` to `spender`.
     pub fn allowance(env: Env, owner: Address, spender: Address) -> i128 {
         read_allowance(&env, &owner, &spender)
     }

     /// Approve `spender` to spend `amount` on behalf of caller.
     pub fn approve(env: Env, owner: Address, spender: Address, amount: i128) -> Result<(), Error> {
         if is_paused(&env) {
             return Err(Error::Paused);
         }
         owner.require_auth();
         write_allowance(&env, &owner, &spender, amount);
         Ok(())
     }

     /// Transfer `amount` from caller to `to`.
     pub fn transfer(env: Env, from: Address, to: Address, amount: i128) -> Result<(), Error> {
         if is_paused(&env) {
             return Err(Error::Paused);
         }
         from.require_auth();
         Self::transfer_internal(&env, &from, &to, amount)
     }

     /// Transfer `amount` from `from` to `to` using allowance granted to `spender`.
     pub fn transfer_from(
         env: Env,
         spender: Address,
         from: Address,
         to: Address,
         amount: i128,
     ) -> Result<(), Error> {
         if is_paused(&env) {
             return Err(Error::Paused);
         }
         spender.require_auth();
         let current_allowance = read_allowance(&env, &from, &spender);
         if current_allowance < amount {
             return Err(Error::InsufficientAllowance);
         }
         write_allowance(&env, &from, &spender, current_allowance - amount);
         Self::transfer_internal(&env, &from, &to, amount)
     }

     fn transfer_internal(env: &Env, from: &Address, to: &Address, amount: i128) -> Result<(), Error> {
         if amount < 0 {
             return Err(Error::InsufficientBalance);
         }
         let from_balance = read_balance(env, from);
         if from_balance < amount {
             return Err(Error::InsufficientBalance);
         }
         let to_balance = read_balance(env, to);
         write_balance(env, from, from_balance - amount);
         write_balance(env, to, to_balance + amount);
         Ok(())
     }

     /// Mint `amount` tokens to `to`. Only callable by admin (issuer).
     pub fn mint(env: Env, admin: Address, to: Address, amount: i128) -> Result<(), Error> {
         let current_admin = read_admin(&env);
         if admin != current_admin {
             return Err(Error::NotAdmin);
         }
         admin.require_auth();
         if amount <= 0 {
             return Ok(());
         }
         let to_balance = read_balance(&env, &to);
         let total = read_total_supply(&env);
         write_balance(&env, &to, to_balance + amount);
         write_total_supply(&env, total + amount);
         Ok(())
     }

     /// Burn `amount` tokens from `from`. Only callable by admin (issuer).
     pub fn burn(env: Env, admin: Address, from: Address, amount: i128) -> Result<(), Error> {
         let current_admin = read_admin(&env);
         if admin != current_admin {
             return Err(Error::NotAdmin);
         }
         admin.require_auth();
         if amount <= 0 {
             return Ok(());
         }
         let from_balance = read_balance(&env, &from);
         if from_balance < amount {
             return Err(Error::InsufficientBalance);
         }
         let total = read_total_supply(&env);
         write_balance(&env, &from, from_balance - amount);
         write_total_supply(&env, total - amount);
         Ok(())
     }

     /// Change admin (issuer). Callable only by current admin.
     pub fn set_admin(env: Env, admin: Address, new_admin: Address) -> Result<(), Error> {
         let current_admin = read_admin(&env);
         if admin != current_admin {
             return Err(Error::NotAdmin);
         }
         admin.require_auth();
         write_admin(&env, &new_admin);
         Ok(())
     }

     /// Pause or unpause transfers and approvals. Callable only by admin.
     pub fn set_paused(env: Env, admin: Address, paused: bool) -> Result<(), Error> {
         let current_admin = read_admin(&env);
         if admin != current_admin {
             return Err(Error::NotAdmin);
         }
         admin.require_auth();
         set_paused(&env, paused);
         Ok(())
     }

     /// Returns the current admin (issuer) address.
     pub fn admin(env: Env) -> Address {
         read_admin(&env)
     }
 }

