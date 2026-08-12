# Snapit

Professional, fast, and reliable **server backup management CLI** for Linux.

Snapit backs up PostgreSQL databases with `pg_dump`, compresses and checksums archives, uploads to S3-compatible object storage, manages schedules via systemd timers, applies retention safely, and restores with explicit confirmation.

```bash
snapit backup database samane
snapit backup group production
snapit schedule sync
```

This repository is under active development.
