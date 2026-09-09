param(
  [string]$RpcUrl = "https://rpc.testnet.minepi.com",
  [string]$NetworkPassphrase = "Pi Testnet",
  # Funding/signing identity for deploy + init calls (must exist and be funded on Pi Testnet)
  [Parameter(Mandatory = $true)][string]$DeployerSecret,
  # Backend env file to update with deployed contract IDs
  [string]$BackendEnvPath = "c:\Users\USER\usdp\Usdp-Mainnet-backend-v1\.env",
  # Required init params for usdp-pool
  [Parameter(Mandatory = $true)][string]$PoolAdminAddress,
  [Parameter(Mandatory = $true)][string]$OracleContractId,
  [Parameter(Mandatory = $true)][string]$PiTokenContractId,
  # Optional: APY for savings (scaled by 1e6; 30000 = 3%)
  [int]$SavingsApyScaled = 30000
)

$ErrorActionPreference = "Stop"

function Require-Command($name) {
  $cmd = Get-Command $name -ErrorAction SilentlyContinue
  if (-not $cmd) { throw "Missing required command '$name'. Install Stellar CLI (soroban) and ensure it's on PATH." }
}

function Write-Info($msg) { Write-Host "[deploy] $msg" }

function Ensure-BackendEnvLine($path, $key, $value) {
  if (-not (Test-Path $path)) { throw "Backend env file not found: $path" }
  $content = Get-Content -Raw $path
  $pattern = "(?m)^\s*$([regex]::Escape($key))\s*=.*$"
  $line = "$key=$value"
  if ($content -match $pattern) {
    $content = [regex]::Replace($content, $pattern, $line)
  } else {
    if (-not $content.EndsWith("`n")) { $content += "`r`n" }
    $content += "$line`r`n"
  }
  Set-Content -NoNewline -Path $path -Value $content
}

Require-Command "stellar"
Require-Command "cargo"

Write-Info "Building WASM artifacts (release)"
cargo build --release

function Deploy-Contract($wasmPath) {
  if (-not (Test-Path $wasmPath)) { throw "WASM not found: $wasmPath" }
  $out = stellar contract deploy `
    --wasm $wasmPath `
    --source-secret-key $DeployerSecret `
    --rpc-url $RpcUrl `
    --network-passphrase $NetworkPassphrase
  $id = ($out | Select-Object -Last 1).Trim()
  if (-not $id) { throw "Failed to parse contract id from deploy output: $out" }
  return $id
}

function Invoke-Contract($contractId, $fn, $args) {
  $cmd = @(
    "contract","invoke",
    "--id",$contractId,
    "--source-secret-key",$DeployerSecret,
    "--rpc-url",$RpcUrl,
    "--network-passphrase",$NetworkPassphrase,
    "--",$fn
  ) + $args
  $out = & stellar @cmd
  return $out
}

$root = Split-Path -Parent $PSScriptRoot

# WASM locations (cargo build outputs)
$wasmToken = Join-Path $root "target\wasm32-unknown-unknown\release\usdp_token.wasm"
$wasmPool = Join-Path $root "target\wasm32-unknown-unknown\release\usdp_pool.wasm"
$wasmSavings = Join-Path $root "target\wasm32-unknown-unknown\release\usdp_savings.wasm"
$wasmIncentives = Join-Path $root "target\wasm32-unknown-unknown\release\usdp_incentives.wasm"

Write-Info "Deploying usdp-token"
$USDP_TOKEN_CONTRACT_ID = Deploy-Contract $wasmToken
Write-Info "usdp-token id: $USDP_TOKEN_CONTRACT_ID"

Write-Info "Initializing usdp-token (admin=$PoolAdminAddress)"
Invoke-Contract $USDP_TOKEN_CONTRACT_ID "initialize" @(
  "--admin",$PoolAdminAddress
)

Write-Info "Deploying usdp-savings"
$USDP_SAVINGS_CONTRACT_ID = Deploy-Contract $wasmSavings
Write-Info "usdp-savings id: $USDP_SAVINGS_CONTRACT_ID"

Write-Info "Initializing usdp-savings (admin=$PoolAdminAddress, usdp_token=$USDP_TOKEN_CONTRACT_ID, apy_scaled=$SavingsApyScaled)"
Invoke-Contract $USDP_SAVINGS_CONTRACT_ID "initialize" @(
  "--admin",$PoolAdminAddress,
  "--usdp_token",$USDP_TOKEN_CONTRACT_ID,
  "--apy_scaled",$SavingsApyScaled
)

Write-Info "Deploying usdp-incentives"
$USDP_INCENTIVES_CONTRACT_ID = Deploy-Contract $wasmIncentives
Write-Info "usdp-incentives id: $USDP_INCENTIVES_CONTRACT_ID"

Write-Info "Initializing usdp-incentives (admin=$PoolAdminAddress)"
Invoke-Contract $USDP_INCENTIVES_CONTRACT_ID "initialize" @(
  "--admin",$PoolAdminAddress
)

Write-Info "Deploying usdp-pool"
$USDP_POOL_CONTRACT_ID = Deploy-Contract $wasmPool
Write-Info "usdp-pool id: $USDP_POOL_CONTRACT_ID"

Write-Info "Initializing usdp-pool (admin=$PoolAdminAddress, oracle=$OracleContractId, pi_token=$PiTokenContractId, usdp_token=$USDP_TOKEN_CONTRACT_ID)"
Invoke-Contract $USDP_POOL_CONTRACT_ID "initialize" @(
  "--admin",$PoolAdminAddress,
  "--oracle",$OracleContractId,
  "--pi_token",$PiTokenContractId,
  "--usdp_token",$USDP_TOKEN_CONTRACT_ID,
  "--backstop_take_rate",0,
  "--max_positions",1000,
  "--min_collateral",0
)

Write-Info "Updating backend .env at $BackendEnvPath"
Ensure-BackendEnvLine $BackendEnvPath "PI_SOROBAN_RPC_URL" $RpcUrl
Ensure-BackendEnvLine $BackendEnvPath "PI_NETWORK_PASSPHRASE" $NetworkPassphrase
Ensure-BackendEnvLine $BackendEnvPath "USDP_TOKEN_CONTRACT_ID" $USDP_TOKEN_CONTRACT_ID
Ensure-BackendEnvLine $BackendEnvPath "USDP_POOL_CONTRACT_ID" $USDP_POOL_CONTRACT_ID
Ensure-BackendEnvLine $BackendEnvPath "USDP_SAVINGS_CONTRACT_ID" $USDP_SAVINGS_CONTRACT_ID
Ensure-BackendEnvLine $BackendEnvPath "USDP_INCENTIVES_CONTRACT_ID" $USDP_INCENTIVES_CONTRACT_ID

Write-Info "Done. Deployed IDs:"
Write-Host "USDP_TOKEN_CONTRACT_ID=$USDP_TOKEN_CONTRACT_ID"
Write-Host "USDP_POOL_CONTRACT_ID=$USDP_POOL_CONTRACT_ID"
Write-Host "USDP_SAVINGS_CONTRACT_ID=$USDP_SAVINGS_CONTRACT_ID"
Write-Host "USDP_INCENTIVES_CONTRACT_ID=$USDP_INCENTIVES_CONTRACT_ID"

