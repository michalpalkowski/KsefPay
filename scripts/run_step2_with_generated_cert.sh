#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NIP="${1:-5260250274}"
OUT_DIR="${KSEF_E2E_CERT_DIR:-$ROOT_DIR/.tmp/ksef-e2e-cert}"
CERT_PATH="$OUT_DIR/cert.pem"
KEY_PATH="$OUT_DIR/key.pem"

mkdir -p "$OUT_DIR"

SUBJECT="/2.5.4.42=A/2.5.4.4=R/serialNumber=TINPL-${NIP}/2.5.4.3=A R/C=PL"

echo "[1/3] Generating self-signed XAdES test certificate for NIP $NIP"
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout "$KEY_PATH" \
  -out "$CERT_PATH" \
  -days 730 \
  -sha256 \
  -subj "$SUBJECT" \
  -addext "basicConstraints=critical,CA:FALSE" \
  -addext "keyUsage=critical,digitalSignature" \
  -sigopt rsa_padding_mode:pss \
  -sigopt rsa_pss_saltlen:-1 \
  >/dev/null 2>&1

echo "[2/3] Exporting env vars for E2E signer"
export KSEF_E2E_CERT_PEM="$CERT_PATH"
export KSEF_E2E_KEY_PEM="$KEY_PATH"

echo "[3/3] Running KSeF E2E step2"
cd "$ROOT_DIR"
cargo test -p ksef-core --test ksef_e2e step2 -- --ignored --nocapture
