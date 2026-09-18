#!/usr/bin/env bash
# Deploy local (gitignored) credentials into /var/backupSystem
# Usage: sudo bash scripts/deploy-config.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${ROOT}/config/local"
DEST="${SNAPIT_HOME:-/var/backupSystem}"

if [[ ! -f "${SRC}/databases.json" ]]; then
  echo "Missing ${SRC}/databases.json — create it from your secrets (never commit)."
  exit 1
fi

mkdir -p "${DEST}"
install -m 0600 "${SRC}/databases.json" "${DEST}/databases.json"
install -m 0600 "${SRC}/storage.json" "${DEST}/storage.json"
install -m 0600 "${SRC}/groups.json" "${DEST}/groups.json"
install -m 0600 "${SRC}/schedules.json" "${DEST}/schedules.json"
chmod 700 "${DEST}"
echo "Deployed configs to ${DEST} (mode 600)"
