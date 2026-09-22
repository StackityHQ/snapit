# Snapit

Professional, fast, and reliable **server backup management CLI** for Linux.

Snapit backs up PostgreSQL databases with `pg_dump`, compresses and checksums archives, uploads to S3-compatible object storage, manages schedules via systemd timers, applies retention safely, and restores with explicit confirmation.

```bash
snapit backup database samane
snapit backup group production
snapit schedule sync
```

---

## Features

- PostgreSQL backups via streaming `pg_dump` → gzip (low memory)
- SHA-256 checksums and archive verification
- Multiple databases and backup groups
- Multiple S3-compatible destinations (Arvan, AWS, MinIO, …)
- Cron-friendly scheduling with systemd timers
- Retention policies (keep last N / keep days) with union protection
- Safe restore workflow with preview + `--yes`
- Structured logs under `/var/log/backup-system`
- SQLite metadata store (not only filenames)
- Secrets never printed in normal CLI output or logs
- Single static-friendly release binary

---

## Architecture

```text
snapit
├── CLI            clap + tables/colors
├── configuration  JSON (databases, storage, groups, schedules)
├── backup engine  pg_dump → gzip → checksum → verify → upload
├── storage engine S3-compatible (path-style supported)
├── scheduler      systemd timers (no permanent daemon)
├── retention      keep_last / keep_days
├── restore        psql stream restore with confirmation
├── metadata       SQLite job/backup history
└── logging        daily rolling logs
```

**Paths (defaults):**

| Purpose | Path |
|---------|------|
| Config | `/var/backupSystem` |
| Data / backups / SQLite | `/var/lib/backup-system` |
| Logs | `/var/log/backup-system` |
| Binary | `/usr/local/bin/snapit` |

Override with `SNAPIT_HOME`, `SNAPIT_DATA`, `SNAPIT_LOG`.

---

## Requirements

- Linux (x86_64 or aarch64)
- `pg_dump` / `psql` (`postgresql-client`)
- systemd (recommended for schedules)
- Network access to PostgreSQL and object storage

---

## Installation

### From release (recommended)

```bash
# Set your GitHub repo, then:
curl -fsSL https://raw.githubusercontent.com/<org>/snapit/v1.0.0/install.sh | \
  sudo SNAPIT_REPO=<org>/snapit SNAPIT_VERSION=v1.0.0 bash
```

Or download the tarball for your architecture from GitHub Releases, extract, and:

```bash
sudo bash install.sh --from ./snapit
```

### From source

```bash
cargo build --release
sudo ./install.sh --from ./target/release/snapit
# or:
sudo ./target/release/snapit install --binary ./target/release/snapit --with-examples
```

The installer:

1. Detects architecture  
2. Installs `/usr/local/bin/snapit`  
3. Creates `/var/backupSystem`, `/var/lib/backup-system`, `/var/log/backup-system`  
4. Creates system user `snapit`  
5. Seeds example JSON configs (`chmod 600`)  
6. Installs a base systemd unit  
7. Verifies the binary  

---

## Configuration

All configuration is human-readable JSON. **Do not commit production credentials.**

### Databases — `/var/backupSystem/databases.json`

```json
{
  "databases": [
    {
      "name": "samane",
      "driver": "postgres",
      "host": "95.38.176.204",
      "port": 5832,
      "database": "postgres",
      "username": "delta",
      "password": "REDACTED",
      "enabled": true,
      "retention": { "keep_last": 14 }
    },
    {
      "name": "deltakonkur",
      "driver": "postgres",
      "host": "95.38.176.204",
      "port": 5832,
      "database": "deltakonkur_db",
      "username": "delta",
      "password": "REDACTED",
      "enabled": true,
      "retention": { "keep_last": 14 }
    }
  ]
}
```

Add as many databases as you need — nothing is hardcoded.

### Storage — `/var/backupSystem/storage.json`

```json
{
  "storages": [
    {
      "name": "arvan",
      "provider": "s3",
      "region": "ir-thr-at1",
      "bucket": "deltakonkur-object-backup-bucket",
      "access_key_id": "REDACTED",
      "secret_access_key": "REDACTED",
      "endpoint": "https://s3.ir-thr-at1.arvanstorage.ir",
      "path_style": true,
      "enabled": true,
      "retention": { "keep_last": 14 }
    }
  ]
}
```

### Groups — `/var/backupSystem/groups.json`

```json
{
  "groups": [
    {
      "name": "production",
      "databases": ["samane", "deltakonkur"],
      "storages": ["arvan"],
      "retention": { "keep_last": 14 },
      "enabled": true
    }
  ]
}
```

### Schedules — `/var/backupSystem/schedules.json`

```json
{
  "schedules": [
    {
      "name": "nightly",
      "when": "02:00",
      "group": "production",
      "storages": ["arvan"],
      "retention": { "keep_last": 14 },
      "enabled": true
    }
  ]
}
```

`when` accepts `hourly`, `daily`, `HH:MM`, or a systemd `OnCalendar` expression.

Secure configs:

```bash
sudo chmod 700 /var/backupSystem
sudo chmod 600 /var/backupSystem/*.json
sudo chown -R snapit:snapit /var/backupSystem /var/lib/backup-system /var/log/backup-system
```

Example templates (no secrets): `config/examples/`.

---

## CLI usage

```bash
snapit status
snapit config show          # secrets redacted
snapit config paths
snapit database list
snapit storage list
snapit storage test arvan
snapit group list
```

### Backups

```bash
# Single database
snapit backup database samane

# Multiple
snapit backup databases samane deltakonkur

# Group
snapit backup group production

# Optional storage override
snapit backup database samane --storage arvan

# Manage
snapit backup list
snapit backup show <id>
snapit backup verify <id>
snapit backup delete <id> --yes
```

Backup names:

```text
samane_2026-09-22_23-30-00.sql.gz
production_2026-09-22_23-30-00/
```

### Restore

```bash
snapit restore <backup-id>
# review preview, then:
snapit restore <backup-id> --target deltakonkur --yes
```

Never silently overwrites — requires `--yes` and an interactive confirmation.

### Schedules

```bash
snapit schedule add nightly --when 02:00 --group production --storages arvan --keep-last 14
snapit schedule list
snapit schedule enable nightly
snapit schedule disable nightly
snapit schedule remove nightly
snapit schedule sync          # write + enable systemd timers
```

### Retention

```bash
snapit retention --database samane
snapit retention --group production
```

Policies can be set per database, group, or storage. A backup is deleted only if **no** applicable policy still requires it.

### Logs

```bash
snapit logs
snapit logs --follow
snapit logs --job <job-id>
snapit logs --lines 200
```

---

## Systemd integration

`snapit schedule sync` writes:

```text
/etc/systemd/system/snapit-schedule-<name>.service
/etc/systemd/system/snapit-schedule-<name>.timer
```

Example timer: every day at 02:00 → backup group `production` → upload to Arvan → retention keep last 14.

```bash
systemctl list-timers 'snapit-schedule-*'
systemctl status snapit-schedule-nightly.timer
journalctl -u snapit-schedule-nightly.service
```

Job locking prevents two identical scheduled jobs from running at once.

---

## Security

- Passwords / access keys never appear in `status`, tables, or `config show`
- Errors are scrubbed for `PGPASSWORD` and similar patterns
- Config files should be `chmod 600`, owned by `snapit`
- Processes are spawned without a shell (no injection via metacharacters)
- Partial backups use `.partial` files and are cleaned up on failure
- Uploads are verified with `HeadObject` size checks
- Failed uploads are **not** reported as successful backups
- Restore requires explicit confirmation

---

## Upgrade

```bash
# From a new release binary
sudo install -m 0755 ./snapit /usr/local/bin/snapit
snapit --version
snapit schedule sync
```

Configuration and metadata under `/var/backupSystem` and `/var/lib/backup-system` are preserved.

---

## Release process

Tag a version (drives the release name — not hardcoded in multiple places):

```bash
git tag v1.0.0
git push origin v1.0.0
```

GitHub Actions will:

1. Run `fmt`, `clippy`, `test`  
2. Build `x86_64` and `aarch64` Linux release binaries  
3. Package `.tar.gz` + SHA-256 checksums  
4. Create a GitHub Release and attach artifacts  

See `.github/workflows/release.yml`.

---

## Development

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Local testing without touching production paths:

```bash
export SNAPIT_HOME=/tmp/snapit-home
export SNAPIT_DATA=/tmp/snapit-data
export SNAPIT_LOG=/tmp/snapit-log
mkdir -p "$SNAPIT_HOME" "$SNAPIT_DATA" "$SNAPIT_LOG"
cp config/examples/*.json "$SNAPIT_HOME/"
# edit passwords, then:
cargo run -- status
```

---

## Testing

Unit tests cover:

- Configuration parsing and validation  
- Secret redaction  
- Backup naming  
- Checksums  
- Retention (keep_last / keep_days / multi-policy union)  
- Schedule OnCalendar conversion  
- Metadata CRUD  
- Lock exclusivity  

Integration against real PostgreSQL/S3 should be run on the target server:

```bash
snapit storage test arvan
snapit backup database samane
snapit backup verify <id>
```

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `failed to load config` | Create JSON under `SNAPIT_HOME` / run `snapit install` |
| `pg_dump not found` | `apt install postgresql-client` |
| Auth failures | Check host/port/user; password never echoed — edit JSON |
| Upload failed | `snapit storage test <name>`; check endpoint + path_style |
| Timer not firing | `snapit schedule sync`; `systemctl list-timers` |
| Lock error | Previous job still running or crashed holding flock — check `locks/` |

---

## Project layout

```text
snapit/
├── src/
│   ├── backup/       dump, compress, verify
│   ├── cli/          commands + UI
│   ├── config/       JSON schemas + paths
│   ├── install/      install helpers
│   ├── logging/
│   ├── metadata/     SQLite
│   ├── restore/
│   ├── retention/
│   ├── schedule/     systemd generation
│   ├── storage/      S3
│   └── util/
├── config/examples/
├── systemd/
├── install.sh
└── .github/workflows/release.yml
```

---

## License

MIT
