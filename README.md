# PUSD Soroban Contracts

This workspace contains Soroban contracts for the PUSD ecosystem on Pi:

- `lending-pool`: over-collateralized DeFi lending protocol (Pi collateral, PUSD borrow).
- `price-oracle`: Pi/USD oracle used by lending.
- `pusd-pool`: request-based PUSD lending pool (Pi collateral, PUSD borrow, interest accrual).
- `pusd-token`: **PUSD standard token contract** with issuer-controlled mint/burn.

**Token standard note:** All contracts treat Pi and PUSD as **Soroban token standard–style tokens**. Lending pools and other dApps **do not mint PUSD**; they only transfer tokens they already hold. PUSD issuance (mint/burn) is handled by the dedicated `pusd-token` contract, which restricts mint/burn to an off-chain–controlled issuer/admin.

---

## PUSD token (`pusd-token`)

The `pusd-token` crate implements the core PUSD asset as a simple, permissionless Soroban token with issuer-only supply control.

- **Name**: `PUSD`
- **Symbol**: `PUSD`
- **Decimals**: `7`
- **Admin/issuer**:
  - Stored in contract state.
  - Set once via `initialize(env, admin)` at deployment.
  - Only the admin can:
    - `mint(admin, to, amount)`
    - `burn(admin, from, amount)`
    - `set_admin(admin, new_admin)`
    - `set_paused(admin, paused)`
- **Permissionless usage**:
  - Any address can hold and transfer PUSD.
  - No on-chain KYC or Pi identity checks; those live entirely in your backend/app if needed.

### Interface

Core read methods:

- `name(env) -> BytesN<32>`
- `symbol(env) -> BytesN<32>`
- `decimals(env) -> u32`
- `total_supply(env) -> i128`
- `balance(env, owner: Address) -> i128`
- `allowance(env, owner: Address, spender: Address) -> i128`
- `admin(env) -> Address`

Core write methods:

- `initialize(env, admin: Address)` – one-time init, sets admin.
- `approve(env, owner: Address, spender: Address, amount: i128) -> Result<(), Error>`
- `transfer(env, from: Address, to: Address, amount: i128) -> Result<(), Error>`
- `transfer_from(env, spender: Address, from: Address, to: Address, amount: i128) -> Result<(), Error>`
- `mint(env, admin: Address, to: Address, amount: i128) -> Result<(), Error>` (admin only)
- `burn(env, admin: Address, from: Address, amount: i128) -> Result<(), Error>` (admin only)
- `set_admin(env, admin: Address, new_admin: Address) -> Result<(), Error>` (admin only)
- `set_paused(env, admin: Address, paused: bool) -> Result<(), Error>` (admin only)

Errors (`Error`):

- `NotAdmin`
- `Paused`
- `InsufficientBalance`
- `InsufficientAllowance`

### Backend integration expectations

The backend (`Usdp-Mainnet-backend-v1`) is the only component that should invoke `mint` and `burn`:

- **Mint path**: after a successful Pi (or other asset) deposit and reserve update, backend calls `mint(admin, user_address, amount)`.
- **Burn path**: when a user returns PUSD (burn for Pi or another asset), backend calls `burn(admin, user_address, amount)` to destroy the tokens.
- **Key management**:
  - Treat the admin key as highly sensitive (ideally multisig/HSM).
  - Use rate limits and operational procedures for high-value mint/burns.
  - Rotate admin key via `set_admin` when needed.

The token contract **does not know about USD reserves, Pi price, or banking integrations**. All reserve accounting and peg maintenance are handled off-chain; on-chain logic remains simple and auditable.

## Lending overview

| Asset | Role | Decimals |
|-------|------|----------|
| Pi | Collateral | 7 |
| PUSD | Borrow (pegged ~$1) | 7 |

**Mechanics:** Users deposit Pi, borrow PUSD (transferred from pool reserves) up to the max LTV. If collateral value falls below the liquidation threshold, positions can be liquidated; the liquidator repays PUSD and receives Pi collateral plus a bonus.

---

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Soroban Contracts (On-Chain)                   │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐     ┌──────────────────┐                    │
│  │  Price Oracle   │◄────│  Lending Pool    │                    │
│  │  (Pi/USD)       │     │  (Pi→PUSD)       │                    │
│  └────────▲────────┘     └────────▲─────────┘                    │
│           │                       │                               │
└───────────┼───────────────────────┼───────────────────────────────┘
            │                       │
┌───────────┼───────────────────────┼───────────────────────────────┐
│           │      PUSD Backend     │                                │
│  ┌────────┴────────┐    ┌────────┴────────┐    ┌────────────────┐ │
│  │ OraclePricePusher│    │ LiquidationMonitor│   │ LendingEvent   │ │
│  │ (set_price)     │    │ (liquidate)       │   │ Monitor        │ │
│  └─────────────────┘    └──────────────────┘    └────────────────┘ │
│           │                       │                       │         │
│           └───────────────────────┴───────────────────────┘         │
│                               │                                     │
│                    LendingProtocolService                           │
│                    LendingPoolClient                                │
└────────────────────────────────────────────────────────────────────┘
```

---

## Contracts

### 1. Price Oracle (`price-oracle`)

Stores the Pi/USD price. The backend pushes updates from the external Oracle API via the `OraclePricePusher` job.

#### Storage

| Key | Type | Description |
|-----|------|-------------|
| `admin` | Address | Admin who can call `set_price` |
| `pi_price` | OracleData | `{ price: i128, updated_at: u64 }` |

#### Data Types

```rust
#[contracttype]
pub struct OracleData {
    pub price: i128,      // Pi price in USD × 10^7 (e.g. 0.25 USD = 2_500_000)
    pub updated_at: u64,  // Ledger timestamp
}
```

#### Public Functions

| Function | Auth | Description |
|----------|------|-------------|
| `initialize(e, admin)` | — | One-time init. Sets admin. |
| `set_price(e, admin, price)` | admin | Sets Pi price. `price` in USD × 10^7. |
| `get_price(e)` | — | Returns `i128` (price × 10^7) |
| `get_oracle_data(e)` | — | Returns full `OracleData` |
| `admin(e)` | — | Returns admin `Address` |

#### Constraints

- `set_price`: `price > 0`
- Storage TTL extended on each `set_price`

---

### 2. Lending Pool (`lending-pool`)

Core lending logic: deposit Pi, borrow/repay PUSD, withdraw collateral, liquidate.

#### Constants

| Name | Value | Meaning |
|------|-------|---------|
| `BPS_SCALE` | 10_000 | Basis points denominator |
| `DECIMALS` | 10_000_000 | 7 decimals for Stellar assets |

#### Data Types

```rust
#[contracttype]
pub struct Position {
    pub collateral: i128,  // Pi amount (7 decimals)
    pub debt: i128,        // PUSD amount (7 decimals)
}

#[contracttype]
pub struct Config {
    pub admin: Address,
    pub pi_token: Address,
    pub pusd_token: Address,
    pub oracle: Address,
    pub max_ltv_bps: i128,       // e.g. 8000 = 80%
    pub liq_threshold_bps: i128, // e.g. 8500 = 85%
    pub liq_bonus_bps: i128,     // e.g. 500 = 5%
    pub paused: bool,
}
```

#### Storage

| Key | Type | Description |
|-----|------|-------------|
| `config` | Config | Pool config (instance storage) |
| `(pos, user)` | Position | User position (persistent) |

Position TTL: 1000 ledgers, extend by 2000.

#### Public Functions

| Function | Auth | Description |
|----------|------|-------------|
| `initialize(e, admin, pi_token, pusd_token, oracle, max_ltv_bps, liq_threshold_bps, liq_bonus_bps)` | — | One-time init. Validates BPS values. |
| `deposit(e, from, amount)` | from | Transfers Pi from user → pool, increases collateral. |
| `withdraw(e, to, amount)` | to | Withdraws Pi if LTV stays ≤ max. |
| `borrow(e, to, amount)` | to | Transfers PUSD from the pool to the user if LTV stays ≤ max (pool must be pre-funded). |
| `repay(e, from, amount)` | from | Repays PUSD debt. |
| `liquidate(e, liquidator, user, repay_amount)` | liquidator | Repays PUSD, seizes Pi + bonus for liquidator. |
| `get_position(e, user)` | — | Returns `Position` for user. |
| `config(e)` | — | Returns pool `Config`. |
| `set_paused(e, admin, paused)` | admin | Pauses/unpauses the pool. |

#### Validation Rules

**initialize**

- `max_ltv_bps`: 0 < x ≤ 10_000
- `liq_threshold_bps`: max_ltv_bps < x ≤ 10_000
- `liq_bonus_bps`: 0 ≤ x ≤ 10_000

**LTV Logic**

- Collateral value: `coll_val = collateral × pi_price / DECIMALS`
- Max debt: `max_debt = coll_val × max_ltv_bps / BPS_SCALE`
- Borrow/reduce collateral only if `new_debt ≤ max_debt`

**Liquidation**

- Liquidatable when: `coll_val < debt × BPS_SCALE / liq_threshold_bps`
- Collateral seized: `collateral_to_seize = repay × DECIMALS / pi_price`
- Bonus: `bonus = collateral_to_seize × liq_bonus_bps / BPS_SCALE`
- Liquidator receives: `collateral_to_seize + bonus`

#### Events (`#[contractevent]`)

| Event | Topics | Data |
|-------|--------|------|
| `DepositEvent` | `["Deposit"]`, `from` | `amount` |
| `WithdrawEvent` | `["Withdraw"]`, `to` | `amount` |
| `BorrowEvent` | `["Borrow"]`, `to` | `amount` |
| `RepayEvent` | `["Repay"]`, `from` | `amount` |
| `LiquidateEvent` | `["Liquidate"]`, `user` | `liquidator`, `repay`, `total_seize` |

---

### 3. PUSD lending pool (`pusd-pool`)

PUSD lending pool: request-based `submit(from, to, requests[])`, reserves (Pi collateral, PUSD borrow), health factor, bToken/dToken-style accrual, and instant liquidation. When `PUSD_POOL_CONTRACT_ID` is set, the backend uses `PUSDPoolClient` for positions and liquidations.

- **Initialize:** `initialize(admin, oracle, pi_token, pusd_token, backstop_take_rate, max_positions, min_collateral)`
- **Actions:** `submit(from, to, requests)` with request types Supply, Withdraw, SupplyCollateral, WithdrawCollateral, Borrow, Repay; `liquidate(liquidator, user, repay_amount)`
- **Getters:** `get_config`, `get_reserve_list`, `get_reserve(asset)`, `get_positions(user)`, `set_paused(admin, paused)`

---

## Build & Test

### Prerequisites

- [Rust](https://rustup.rs/) with `wasm32-unknown-unknown` target
- [Stellar CLI](https://developers.stellar.org/docs/build/smart-contracts/getting-started/setup) (for deployment)

### Build

```bash
cd contracts
cargo build --target wasm32-unknown-unknown --release
```

**Output**

- `lending-pool/target/wasm32-unknown-unknown/release/soroban_lending_pool_contract.wasm`
- `price-oracle/target/wasm32-unknown-unknown/release/soroban_price_oracle_contract.wasm`

### Test

```bash
cargo test
```

---

## Deployment

### Order

1. Deploy **price-oracle** and `initialize(admin)`.
2. Deploy **lending-pool** and `initialize(admin, pi_token, pusd_token, oracle, max_ltv_bps, liq_threshold_bps, liq_bonus_bps)`.
3. Start backend jobs to push oracle price and monitor events.
4. Fund the pool with PUSD (transfer to pool contract) so users can borrow.

### Example Config

- `max_ltv_bps`: 8000 (80%)
- `liq_threshold_bps`: 8500 (85%)
- `liq_bonus_bps`: 500 (5%)

### Token Addresses

- `pi_token`: Stellar Asset Contract (SAC) address for Pi
- `pusd_token`: SAC address for PUSD

### Deploying to Pi Testnet

The same Soroban contracts run on Pi Network (Pi uses Soroban like Stellar). To deploy and use the backend with Pi testnet:

1. **Set Pi RPC and passphrase** (in `.env` or environment):
   - `PI_SOROBAN_RPC_URL` – Pi testnet Soroban RPC URL (e.g. Pi’s public testnet RPC).
   - `PI_TESTNET_PASSPHRASE` or `PI_NETWORK_PASSPHRASE` – Pi testnet network passphrase (e.g. `Pi Testnet`).

2. **Build** (unchanged):  
   `cargo build --target wasm32-unknown-unknown --release` from `contracts/`.

3. **Deploy** using Stellar/Soroban CLI pointed at Pi:
   - Configure the CLI to use `PI_SOROBAN_RPC_URL` and the Pi passphrase (e.g. `soroban contract deploy` with `--network` or env).
   - Deploy **price-oracle**, then **lending-pool** or **pusd-pool**; call each contract’s `initialize(...)` with admin, oracle, and token addresses.

4. **Backend**: Set `LENDING_POOL_CONTRACT_ID` and/or `PUSD_POOL_CONTRACT_ID` (and `LENDING_ORACLE_CONTRACT_ID`, `LENDING_ADMIN_SECRET_KEY`) to the deployed contract IDs. The backend uses `getSorobanServer()` and `getNetworkPassphrase()` from `src/lib/soroban/rpc.ts`, which read `PI_SOROBAN_RPC_URL` and `PI_TESTNET_PASSPHRASE` when set, so no code change is required—only deployment target and env.

---

## Backend Integration

### Components

| Component | Location | Role |
|-----------|----------|------|
| `LendingPoolClient` | `src/lib/soroban/LendingPoolClient.ts` | Legacy pool: position, oracle, liquidate, setOraclePrice |
| `PUSDPoolClient` | `src/lib/soroban/PUSDPoolClient.ts` | PUSD lending pool: submit, getPositions, getReserve, liquidate |
| `LendingProtocolService` | `src/services/LendingProtocolService.ts` | Combines on-chain + off-chain logic; uses PUSD pool when `PUSD_POOL_CONTRACT_ID` set |
| `OraclePricePusher` | `src/jobs/oraclePricePusher.ts` | Pushes Pi price from Oracle API to Price Oracle |
| `LiquidationMonitor` | `src/jobs/liquidationMonitor.ts` | Finds and liquidates undercollateralized positions |
| `LendingEventMonitor` | `src/services/LendingEventMonitor.ts` | Polls events and syncs to DB |

### Event Polling

`LendingEventMonitor` listens for events from the Lending Pool contract:

- Filters: `type: 'contract'`, `contractIds: [LENDING_POOL_CONTRACT_ID]`
- Event types: `Deposit`, `Borrow`, `Repay`, `Withdraw`, `Liquidate` (legacy); `Supply`, `SupplyCollateral`, `WithdrawCollateral`, `LiquidationAuctionCreated`, `LiquidationAuctionFilled` (PUSD pool)
- Topics: `[eventType, userAddress]` (first topic = event name, second = user)
- When `PUSD_POOL_CONTRACT_ID` is set, the event monitor uses that contract ID; otherwise `LENDING_POOL_CONTRACT_ID`
- Updates `LendingPosition` and creates `LendingEvent` records for affected users

### Database Models

**LendingPosition**

- `userAddress` (unique)
- `piCollateralAmount`, `pusdDebtAmount` (human-readable)
- `lastUpdated`

**LendingEvent**

- `eventType`, `userAddress`, `txHash`, `amount`, `metadata`, `timestamp`

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `PI_SOROBAN_RPC_URL` | Soroban RPC URL (Pi testnet or `https://soroban-testnet.stellar.org`) |
| `PI_TESTNET_PASSPHRASE` | Pi testnet network passphrase (used with Pi RPC) |
| `LENDING_POOL_CONTRACT_ID` | Deployed legacy LendingPool contract ID |
| `PUSD_POOL_CONTRACT_ID` | Deployed PUSD lending pool contract ID (optional; when set, lending uses PUSD pool) |
| `LENDING_ORACLE_CONTRACT_ID` | Deployed PriceOracle contract ID |
| `LENDING_ADMIN_SECRET_KEY` | Keypair for oracle updates and liquidations |
| `LENDING_LIQUIDATION_INTERVAL_MS` | Liquidation job interval (default: 60000) |
| `LENDING_EVENT_POLL_INTERVAL_MS` | Event monitor interval (default: 15000) |
| `LENDING_ORACLE_PUSH_INTERVAL_MS` | Oracle push interval (default: 60000) |

Backend also uses `ORACLE_API_URL` and `ORACLE_PI_PRICE_ENDPOINT` for the Pi price source.

---

## API Endpoints (Backend)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/lending/health` | Lending protocol health |
| GET | `/api/lending/position/:address` | Position for address |
| GET | `/api/lending/position` | Auth: current user's position |
| GET | `/api/lending/events` | Lending events (filtered) |
| GET | `/api/lending/events/:address` | Events for address |

---

## Security Notes

- Oracle admin must be kept secure; it can change prices.
- Liquidator key (`LENDING_ADMIN_SECRET_KEY` or equivalent) must be protected.
- Pool can be paused with `set_paused` in emergencies.
- Ensure oracle price is pushed frequently to avoid stale prices.

---

## Workspace Layout

```
contracts/
├── Cargo.toml           # Workspace root, release profile
├── rust-toolchain.toml  # stable, wasm32-unknown-unknown
├── lending-pool/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       └── test.rs
├── pusd-pool/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── price-oracle/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       └── test.rs
└── README.md            # This file
```
