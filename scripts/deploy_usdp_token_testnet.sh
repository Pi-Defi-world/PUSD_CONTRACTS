 #!/usr/bin/env bash
 set -euo pipefail

 # Deploy the USDP token contract to the configured Soroban/Pi testnet.
 #
 # Requirements:
 # - soroban CLI installed and configured
 # - PI_SOROBAN_RPC_URL and PI_TESTNET_PASSPHRASE (or equivalent network config) exported
 # - ADMIN_SECRET_KEY set to the issuer/admin keypair
 #
 # This script builds the usdp-token contract, deploys it, initializes admin,
 # and writes the resulting contract ID to deployments/usdp-token.testnet.json.

 ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
 CONTRACT_PACKAGE="usdp-token"
 DEPLOYMENTS_DIR="${ROOT_DIR}/deployments"

 mkdir -p "${DEPLOYMENTS_DIR}"

 echo "Building ${CONTRACT_PACKAGE} for wasm32-unknown-unknown..."
 (cd "${ROOT_DIR}" && cargo build -p "${CONTRACT_PACKAGE}" --target wasm32-unknown-unknown --release)

 WASM_PATH="${ROOT_DIR}/target/wasm32-unknown-unknown/release/${CONTRACT_PACKAGE}.wasm"

 if [ ! -f "${WASM_PATH}" ]; then
   echo "WASM not found at ${WASM_PATH}"
   exit 1
 fi

 if [ -z "${ADMIN_SECRET_KEY:-}" ]; then
   echo "ADMIN_SECRET_KEY environment variable must be set to the issuer keypair."
   exit 1
 fi

 echo "Deploying ${CONTRACT_PACKAGE}..."
 CONTRACT_ID=$(soroban contract deploy \
   --wasm "${WASM_PATH}" \
   --source "${ADMIN_SECRET_KEY}" \
   --network testnet)

 echo "Deployed USDP token contract with ID: ${CONTRACT_ID}"

 echo "Initializing admin..."
 soroban contract invoke \
   --id "${CONTRACT_ID}" \
   --source "${ADMIN_SECRET_KEY}" \
   --network testnet \
   --fn initialize \
   --arg admin="${ADMIN_SECRET_KEY}"

 cat > "${DEPLOYMENTS_DIR}/usdp-token.testnet.json" <<EOF
 {
   "network": "testnet",
   "contract_id": "${CONTRACT_ID}"
 }
 EOF

 echo "Wrote deployment info to ${DEPLOYMENTS_DIR}/usdp-token.testnet.json"

