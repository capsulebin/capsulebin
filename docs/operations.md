# Operations handbook

Running a CapsuleBin instance: deployment, configuration that matters, routine tasks, and what to
do when something happens. Every setting is documented in
[`capsulebin.example.toml`](../capsulebin.example.toml); the reasoning behind the security
properties is in the [threat model](threat-model.md).

## What a deployment consists of

- **One binary** (`capsulebin`) — server, embedded web client, migrations and operator tools.
- **One SQLite database** on a local filesystem (WAL mode; not NFS).
- **Optionally a configuration file**; every setting also has an environment variable
  `CAPSULEBIN__<TABLE>__<KEY>`.
- **A reverse proxy** in front for TLS and HSTS. CapsuleBin speaks plain HTTP.

No other service is required. The [CLI](../README.md#command-line-client) is a separate Node.js
tool for users and automation.

## First setup

1. Generate a creation token and configure its hash (the token itself is never configured):

   ```sh
   capsulebin token
   ```

   Hand the token to the people or systems that may create capsules. Several hashes may be
   configured; remove one to revoke that token.

2. Write the configuration. The minimum for production:

   ```toml
   [server]
   bind = "127.0.0.1:8080"
   public_url = "https://paste.example.org"
   trusted_proxies = ["127.0.0.1/32"]   # the reverse proxy's own address

   [database]
   path = "/var/lib/capsulebin/capsulebin.db"

   [creation]
   mode = "token"
   token_hashes = ["sha256:…"]

   [abuse]
   contact = "abuse@example.org"
   ```

3. Verify before opening a port, then start:

   ```sh
   capsulebin --config /etc/capsulebin.toml check
   capsulebin --config /etc/capsulebin.toml serve
   ```

> **Known limitation:** the web client's create page does not send a creation token yet. In
> `token` mode, capsules can only be created with the CLI (`CAPSULEBIN_TOKEN` or `--token-file`);
> the web client can still open and manage them. `public` mode makes the web create page work but
> lets anyone upload — only sensible behind another access control (VPN, SSO proxy).

### systemd

```ini
[Unit]
Description=CapsuleBin
After=network.target

[Service]
ExecStart=/usr/local/bin/capsulebin --config /etc/capsulebin.toml serve
User=capsulebin
Group=capsulebin
StateDirectory=capsulebin
UMask=0077
Restart=on-failure
# SIGTERM: stop accepting connections, finish in-flight requests.
KillSignal=SIGTERM
TimeoutStopSec=30

NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/lib/capsulebin
ProtectKernelTunables=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
LockPersonality=true
MemoryDenyWriteExecute=true

[Install]
WantedBy=multi-user.target
```

`UMask=0077` keeps the database and its `-wal`/`-shm` readable by the service user only.

### Reverse proxy

The proxy terminates TLS, sets HSTS and passes the client address in `X-Forwarded-For`. CapsuleBin
already sets the CSP, `Referrer-Policy`, `X-Frame-Options`, `X-Content-Type-Options`,
`Permissions-Policy` and `X-Robots-Tag` on every response — do not override them.

Caddy:

```
paste.example.org {
    header Strict-Transport-Security "max-age=63072000; includeSubDomains"
    reverse_proxy 127.0.0.1:8080
}
```

The request path contains paste ids. Either do not enable an access log on the proxy, or configure
it without the URI and with short retention.

nginx:

```nginx
server {
    listen 443 ssl http2;
    server_name paste.example.org;
    add_header Strict-Transport-Security "max-age=63072000; includeSubDomains" always;
    client_max_body_size 16m;   # above pastes.max_plaintext_size × 4/3
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header Host $host;
    }
    access_log off;   # or a format without $request_uri
}
```

**`trusted_proxies` must list the proxy, not the clients.** Too broad a range lets clients pick
their own address for rate limiting by sending `X-Forwarded-For`; empty behind a proxy makes every
client share the proxy's bucket.

## Settings that matter

| Setting | Recommendation |
| --- | --- |
| `creation.mode` | `token` (default). `public` only behind another access control. |
| `pastes.default_expiry` / `maximum_expiry` / `allow_permanent` | Short defaults keep the instance small; permanent capsules are off by default. |
| `rate_limits.*` | All off by default except `invalid_capability_per_paste` (wrong passwords per capsule, 10/min burst 10), which does not depend on `trusted_proxies`. Once `trusted_proxies` is right, a reasonable start: `create` 5/min burst 2, `bytes_per_day = "100MiB"`, `max_concurrent = 2`; `descriptor` 60/min burst 10; `retrieve` 30/min burst 5; `delete` 10/min burst 3; `report` 5/hour burst 2; `invalid_capability` 10/min burst 10. |
| `purge.*` | Leave enabled. Never-retrieved capsules go after 30 days, inactive ones after 90. |
| `abuse.contact`, `abuse.report_retention` | Set a contact; keep closed reports no longer than needed (default 90 days). |
| `database.synchronous` | `full` (default). Lower only if you accept losing acknowledged writes on power loss. |

## Health and logs

- `GET /health/live` — the process runs.
- `GET /health/ready` — configuration, database, schema and embedded assets are usable. Use this
  for load balancer checks.
- Logs go to stderr. `CAPSULEBIN_LOG` sets the filter (default `info,tower_http=info,sqlx=warn`).
  The access log contains method, redacted path (`/api/v1/pastes/{id}`), status and latency —
  never paste ids, bodies, tokens or report text.

## Routine tasks

### Backups

```sh
capsulebin --config /etc/capsulebin.toml backup --output /backups/capsulebin-$(date +%F).db
```

Safe while serving. The file is consistent, compacted and `0600`. Store backups as carefully as the
database: they contain ciphertext, verifiers and abuse reports with contact details. Rotate them —
an old backup is where deleted capsules survive the longest. Never copy the live database file
directly.

### Restore

```sh
systemctl stop capsulebin
capsulebin --config /etc/capsulebin.toml restore --input /backups/capsulebin-2026-11-17.db --dry-run
capsulebin --config /etc/capsulebin.toml restore --input /backups/capsulebin-2026-11-17.db --force
systemctl start capsulebin
```

`--force` moves the current database and its `-wal`/`-shm` to `<file>.pre-restore-<timestamp>`;
delete that once the restored instance is confirmed working. Restore only backups you made
yourself.

### Upgrades

1. `capsulebin backup --output …` with the running version.
2. Stop the service, replace the binary.
3. `capsulebin --config … check` — applies pending migrations and verifies schema and assets. A
   binary older than the database refuses to run rather than writing rows the schema cannot hold.
4. Start the service and check `/health/ready`.

Migrations only move forward. To roll back, restore the backup from step 1 with the old binary.

### Retention

The purge job runs at start and then daily; `capsulebin prune` applies it once on demand (for
example after lowering a threshold). It also removes closed abuse reports and moderation records
older than `abuse.report_retention`.

### Rotating creation tokens

Generate a new token with `capsulebin token`, add its hash, hand it out, then remove the old hash
and restart. Existing capsules are unaffected — the token only authorizes creation.

## Abuse handling runbook

Reports arrive through `/report`. Only reports about capsules that currently exist reach the queue.

```sh
capsulebin reports list                             # open reports, oldest first
capsulebin reports show <report-id>
capsulebin reports review <report-id>               # optional: mark as in progress
capsulebin moderation status <paste-id>             # lifecycle, size, earlier actions
capsulebin moderation disable <paste-id> --reason <code> --note "<ticket>"
capsulebin moderation delete <paste-id> --reason <code>
capsulebin moderation restore <paste-id>            # only after disable
capsulebin reports close <report-id> --resolution action_taken|no_action|duplicate|invalid
```

- **Disable first** when in doubt: it blocks the capsule immediately and is reversible. Delete is
  final.
- **Do not open reported content** to "check it". Nothing in the tooling decrypts, and a report
  never contains the key. For severe cases (e.g. suspected child sexual abuse material), disable,
  preserve the technical metadata (`moderation status --json`), and involve legal counsel or the
  authorities rather than handling content yourself.
- A moderated capsule looks exactly like an unknown one to everybody; there is no notice to the
  creator or recipients.
- Legal obligations (response times, notification duties) depend on the operator and the
  jurisdiction and are not encoded in the software.

## Incidents

| Situation | Action |
| --- | --- |
| A creation token leaked | Remove its hash, restart. Consider `creation.mode = "disabled"` while investigating — existing capsules stay readable. |
| Flood of uploads | Enable or tighten `rate_limits.create` (rate, `bytes_per_day`, `max_concurrent`); `creation.mode = "disabled"` stops all creation. |
| Database or backup stolen | Content stays protected without the links. Reports in it contain contact data — treat it as a personal-data breach. Rotate creation tokens. |
| Server compromised | Assume served JavaScript may have been modified: web users of that period may have exposed keys. Rebuild from a trusted binary, rotate tokens, inform users. CLI users were not exposed through served code. |
| Disk full | SQLite refuses writes; reads continue. Free space, run `prune`, check `pastes.max_plaintext_size` and the byte quota. |
| `/health/ready` reports a schema mismatch | The binary and the database disagree. Run `check` with the matching binary; never edit `_sqlx_migrations` by hand. |
