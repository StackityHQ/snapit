#!/usr/bin/env bash
# Snapit installer — Ubuntu / Debian Linux
# Usage:
#   curl -fsSL https://github.com/<org>/snapit/releases/latest/download/install.sh | sudo bash
# Or from a release tarball / local build:
#   sudo bash install.sh [--from /path/to/snapit]

set -euo pipefail

REPO="${SNAPIT_REPO:-}"
VERSION="${SNAPIT_VERSION:-latest}"
INSTALL_PREFIX="/usr/local/bin"
CONFIG_DIR="/var/backupSystem"
DATA_DIR="/var/lib/backup-system"
LOG_DIR="/var/log/backup-system"
USER_NAME="snapit"

red() { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
bold() { printf '\033[1m%s\033[0m\n' "$*"; }

need_root() {
  if [[ "${EUID}" -ne 0 ]]; then
    red "Please run as root (sudo)."
    exit 1
  fi
}

detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64) echo "x86_64-unknown-linux-gnu" ;;
    aarch64|arm64) echo "aarch64-unknown-linux-gnu" ;;
    *) red "Unsupported architecture: $(uname -m)"; exit 1 ;;
  esac
}

install_binary_from_release() {
  local arch target url tmp
  arch="$(detect_arch)"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  if [[ -n "${1:-}" && -f "$1" ]]; then
    install -m 0755 "$1" "${INSTALL_PREFIX}/snapit"
    return
  fi

  if [[ -z "${REPO}" ]]; then
    # Fallback: copy currently built binary next to this script if present
    local here
    here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    if [[ -x "${here}/target/release/snapit" ]]; then
      install -m 0755 "${here}/target/release/snapit" "${INSTALL_PREFIX}/snapit"
      return
    fi
    red "Set SNAPIT_REPO=owner/repo or pass --from /path/to/snapit"
    exit 1
  fi

  if [[ "${VERSION}" == "latest" ]]; then
    url="https://github.com/${REPO}/releases/latest/download/snapit-${arch}.tar.gz"
  else
    url="https://github.com/${REPO}/releases/download/${VERSION}/snapit-${arch}.tar.gz"
  fi

  bold "Downloading ${url}"
  curl -fsSL "$url" -o "${tmp}/snapit.tar.gz"
  tar -xzf "${tmp}/snapit.tar.gz" -C "$tmp"
  install -m 0755 "${tmp}/snapit" "${INSTALL_PREFIX}/snapit"
}

create_user_and_dirs() {
  if ! id -u "${USER_NAME}" >/dev/null 2>&1; then
    useradd --system --home-dir "${DATA_DIR}" --shell /usr/sbin/nologin --user-group "${USER_NAME}"
  fi
  mkdir -p "${CONFIG_DIR}" "${DATA_DIR}/backups" "${DATA_DIR}/locks" "${LOG_DIR}"
  chmod 700 "${CONFIG_DIR}"
  chmod 750 "${DATA_DIR}"
  chmod 755 "${LOG_DIR}"
}

seed_config() {
  local examples
  examples="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/config/examples"
  if [[ ! -f "${CONFIG_DIR}/databases.json" ]]; then
    if [[ -d "${examples}" ]]; then
      cp "${examples}/databases.json" "${CONFIG_DIR}/databases.json"
      cp "${examples}/storage.json" "${CONFIG_DIR}/storage.json"
      cp "${examples}/groups.json" "${CONFIG_DIR}/groups.json"
      cp "${examples}/schedules.json" "${CONFIG_DIR}/schedules.json"
    else
      echo '{"databases":[]}' > "${CONFIG_DIR}/databases.json"
      echo '{"storages":[]}' > "${CONFIG_DIR}/storage.json"
      echo '{"groups":[]}' > "${CONFIG_DIR}/groups.json"
      echo '{"schedules":[]}' > "${CONFIG_DIR}/schedules.json"
    fi
    chmod 600 "${CONFIG_DIR}"/*.json
  fi
  chown -R "${USER_NAME}:${USER_NAME}" "${CONFIG_DIR}" "${DATA_DIR}" "${LOG_DIR}"
}

install_systemd() {
  cat > /etc/systemd/system/snapit-runner.service <<'EOF'
[Unit]
Description=Snapit status / oneshot runner
After=network-online.target

[Service]
Type=oneshot
User=snapit
Group=snapit
Environment=SNAPIT_HOME=/var/backupSystem
Environment=SNAPIT_DATA=/var/lib/backup-system
Environment=SNAPIT_LOG=/var/log/backup-system
ExecStart=/usr/local/bin/snapit status
EOF
  systemctl daemon-reload || true
}

verify() {
  if ! command -v snapit >/dev/null; then
    red "snapit binary not found on PATH"
    exit 1
  fi
  if ! command -v pg_dump >/dev/null; then
    red "warning: pg_dump not found — install postgresql-client"
  fi
  snapit --version || true
  green "Installation verified"
}

main() {
  need_root
  bold "Installing Snapit"
  local from=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --from) from="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  install_binary_from_release "${from}"
  create_user_and_dirs
  seed_config
  install_systemd
  verify
  echo
  green "Snapit installed successfully"
  echo
  echo "Next steps:"
  echo "  1. snapit config init"
  echo "  2. snapit database add <name> --host … --database … --username … --password …"
  echo "  3. snapit storage add <name> --region … --bucket … --endpoint …"
  echo "  4. snapit database list"
  echo "  5. snapit backup database <name>"
  echo "  6. snapit schedule sync"
  echo
  echo "A database.json in the current project directory is used automatically."
}

main "$@"
