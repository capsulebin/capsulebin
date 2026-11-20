# Operations handbook

Running a CapsuleBin instance: deployment, configuration that matters, routine tasks, and what to
do when something happens. Every setting is documented in
[`capsulebin.example.toml`](../capsulebin.example.toml); the reasoning behind the security
properties is in the [threat model](threat-model.md).

## What a deployment consists of

- **One binary** (`capsulebin`) — server, embedded web client, migrations and operator tools —
  run directly (see [systemd](#systemd)) or as the [container image](#container).
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

   [legal.operator]                       # the imprint; see "Legal pages"
   file = "/etc/capsulebin/legal/operator.md"
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

### Container

Releases publish `ghcr.io/capsulebin/capsulebin:<version>` (also tagged `<major>.<minor>`,
`<major>` and `latest`) for `linux/amd64` and `linux/arm64`; how to verify its signature is in the
[README](../README.md#installation). The `Dockerfile` at the repository root builds the same image
locally: it builds the web client, embeds it into the binary and copies the result into a
distroless image without OpenSSL — no shell, no package manager, user `65532`. It is configured through
environment variables only and keeps the database on the `/data` volume.

```sh
docker pull ghcr.io/capsulebin/capsulebin:1.0.0   # or: docker build -t capsulebin .
docker tag ghcr.io/capsulebin/capsulebin:1.0.0 capsulebin
docker run --rm capsulebin token                  # creation token + hash
docker run -d --name capsulebin -p 127.0.0.1:8080:8080 -v capsulebin-data:/data \
  -e CAPSULEBIN__SERVER__PUBLIC_URL=https://paste.example.org \
  -e CAPSULEBIN__SERVER__TRUSTED_PROXIES='["172.16.0.0/12"]' \
  -e CAPSULEBIN__CREATION__TOKEN_HASHES='["sha256:…"]' \
  -e CAPSULEBIN__ABUSE__CONTACT=abuse@example.org \
  capsulebin
```

- The image sets `CAPSULEBIN__SERVER__BIND=0.0.0.0:8080` and
  `CAPSULEBIN__DATABASE__PATH=/data/capsulebin.db`. A configuration file can be mounted instead and
  passed as `--config /path serve`.
- Environment variables are meant for short, single-line values. Put long or multi-line texts,
  especially the [legal texts](#legal-pages), into a configuration file or a Markdown file referenced by `file`,
  mount it read-only and pass it at start (`--config /etc/capsulebin/capsulebin.toml serve`).
  An environment value is taken verbatim as a string (unless it reads as a boolean, an integer or
  an array): quotes and escape sequences such as `\n` are not interpreted, so Markdown passed this
  way does not come out as written.
- `trusted_proxies` must name the address the proxy connects *from* as the container sees it —
  with a published port that is usually the bridge gateway, not `127.0.0.1`. Check the peer
  address in the access log once and narrow the range to it.
- The image's health check runs `capsulebin healthcheck`, which probes `/health/ready` from inside
  the container (see [Health and logs](#health-and-logs)). Podman drops `HEALTHCHECK` from images
  in its default OCI format; build with `podman build --format docker` to keep it.
- Logs go to stderr without colour codes, so `docker logs` shows plain lines.
- Operator commands run in the container: `docker exec capsulebin capsulebin reports list`.
  Backups go to the volume, e.g. `docker exec capsulebin capsulebin backup --output
  /data/backup-2026-11-18.db`; copy them off the host from there.
- Restore with the container stopped, from a one-off container on the same volume:
  `docker run --rm -v capsulebin-data:/data capsulebin restore --input /data/backup-….db --force`.

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

Pages and the redirect away from the default language's prefix (see
[Interface languages](#interface-languages)) are sent with `Cache-Control: no-store`; a caching
proxy or CDN must honour it. No page response depends on `Accept-Language`, so nothing has to be
keyed on it.

**`trusted_proxies` must list the proxy, not the clients.** Too broad a range lets clients pick
their own address for rate limiting by sending `X-Forwarded-For`; empty behind a proxy makes every
client share the proxy's bucket.

## Settings that matter

| Setting | Recommendation |
| --- | --- |
| `server.public_url` | The address users reach, without a trailing slash. It must be a valid http(s) URL; text that does not parse as one is refused at startup. Use a host of its own with the instance at `/` (`https://paste.example.org`): the web interface loads `/assets/…`, calls `/api/…` and creates links directly under the origin, so it does not work below a path. A query or fragment, also an empty one (`https://example.org/team?`, `…#`), is refused at startup: paths are appended to the URL as text. Its path must not end in an interface language code (`https://example.org/team/de` is refused at startup, as is a path that only ends in one once dot segments are resolved, `…/team/de/.`): a link below it could not be told apart from the `/de/` language prefix. |
| `creation.mode` | `token` (default). `public` only behind another access control. |
| `pastes.default_expiry` / `maximum_expiry` / `allow_permanent` | Short defaults keep the instance small; permanent capsules are off by default. |
| `rate_limits.*` | All off by default except `invalid_capability_per_paste` (wrong passwords per capsule, 10/min burst 10), which does not depend on `trusted_proxies`. Once `trusted_proxies` is right, a reasonable start: `create` 5/min burst 2, `bytes_per_day = "100MiB"`, `max_concurrent = 2`; `descriptor` 60/min burst 10; `retrieve` 30/min burst 5; `delete` 10/min burst 3; `report` 5/hour burst 2; `invalid_capability` 10/min burst 10. |
| `purge.*` | Leave enabled. Never-retrieved capsules go after 30 days, inactive ones after 90. |
| `abuse.contact`, `abuse.report_retention` | Set a contact; keep closed reports no longer than needed (default 90 days). |
| `legal.*` | A public instance in the EU usually needs an imprint and a privacy notice. Set `[legal.operator]`; see [Legal pages](#legal-pages). |
| `branding.*` | Optional: name, tagline, logo and favicon of the instance; `source_url` if you run a modified version. See [Branding](#branding). |
| `database.synchronous` | `full` (default). Lower only if you accept losing acknowledged writes on power loss. |

## Legal pages

CapsuleBin serves an imprint, a privacy notice and terms of use at `/imprint`, `/privacy` and
`/terms` in the default interface language and under the prefix of every other one
(`/de/imprint`), and links them from the footer of every page, once `[legal.operator]` is set.
Page titles and headings follow the page language.

- **Imprint**: the operator's own Markdown (`text` or `file`), followed by a "Hosting" block from
  `[legal.hosting]` if someone else hosts the instance.
- **Privacy notice and terms**: built in, in English and German, always in the page's language.
  The privacy notice states what CapsuleBin processes and how long it keeps it, taken from the
  running configuration (retention rules, report retention, whether limits use the client
  address), and names the hosting provider as processor. What it cannot know - the reverse proxy's logs,
  backups, a data protection officer - goes into `[legal.privacy] extra`. Either text can be
  replaced entirely with `text`, `file` or `url`.

`[legal.*]` holds the texts of the default language (`i18n.default_language`). For the other
language, give `[legal.<lang>.operator]`, `.hosting`, `.privacy` and `.terms` with the same keys
(env `CAPSULEBIN__LEGAL__DE__OPERATOR__FILE`):

```toml
[legal.operator]                     # default language (en)
file = "/etc/capsulebin/legal/operator.md"

[legal.de.operator]                  # German pages
file = "/etc/capsulebin/legal/operator.de.md"
```

- A section without a translation falls back: German pages show the `[legal.*]` text, marked with
  the default language's `lang` for screen readers. The built-in texts need no translation.
- A language section replaces its `[legal.*]` counterpart as a whole: an empty `[legal.de.privacy]`
  gives German pages the built-in German notice without the `[legal.privacy]` text or `extra`;
  `[legal.de.privacy] extra` is the German addition. A `url` per language
  (`[legal.de.operator] url`) changes that language's footer links and its privacy notice's link
  to the controller.
- Startup errors: `[legal.<default language>]` (use `[legal.*]`), an unknown language code, and
  `[legal.<lang>.operator]` or `.hosting` without `[legal.operator]` or `[legal.hosting]` - every
  language names the same operator and hosting provider. The rules for a section also apply to what
  a language falls back to: `[legal.de.operator] url` with a `[legal.hosting]` is refused, as
  `[legal.operator] url` is.

The texts are read and rendered at startup, so `capsulebin check` reports a missing file, and a
changed file takes effect on restart. In the container, mount them read-only, e.g.
`-v ./legal:/etc/capsulebin/legal:ro` with `CAPSULEBIN__LEGAL__OPERATOR__FILE=/etc/capsulebin/legal/operator.md`.
Give the texts as files (or `text` in a mounted configuration file), not as `…__TEXT` environment
variables: those are taken verbatim and are unsuitable for multi-line Markdown (see
[Container](#container)).
The built-in texts are a careful default, not legal advice: you are responsible for what your
instance publishes. [privacy-notice.md](privacy-notice.md) lists what they state.

## Interface languages

Every page is served in English and German; both are always offered. The pages of the default
language have no path prefix (`/`, `/p/{id}`, `/report`), the other language lives under its
prefix (`/de/`, `/de/p/{id}`, `/de/report`).

```toml
[i18n]
default_language = "en"   # or "de"; env CAPSULEBIN__I18N__DEFAULT_LANGUAGE
```

`default_language` is the only key in `[i18n]`; a code that is not a known language or any other
key there is a startup error (`capsulebin check` reports it). With `"de"`, German is prefix-less
and English is under `/en/`.

- Paths under the prefix of the default language (`/en/p/{id}` with the default `en`) are
  redirected (`302`) to the same path without it.
- The server does not read `Accept-Language`. In the browser, a language the user picked in the
  switch (kept in `localStorage`), else a supported browser language, moves the page to that
  language; the key in the fragment stays in the browser.
- Share and management links are always created without a prefix, so they open in the default
  language first. Changing `default_language` later does not break them, but moves the pages of
  the previous default under its prefix.
- Any other prefix is an unknown path (`404`).
- `server.public_url` must not end in a language code (see
  [Settings that matter](#settings-that-matter)); the CLI's `put` refuses such a `--server` by
  the same rule. For an existing instance with such an address see [Upgrades](#upgrades).
- The footer link labels and the legal pages follow the page language (see
  [Legal pages](#legal-pages)); the CLI and the API's error texts stay English.

See [i18n.md](i18n.md) for the rules in detail.

## Branding

`[branding]` shows the instance under your own name, logo and favicon, in every page and every
interface language, and can replace the slogan on the create page. Without it the pages say
"CapsuleBin" and use the built-in symbol, slogan and favicon.

```toml
[branding]
name = "Example Paste"                                  # CAPSULEBIN__BRANDING__NAME
logo = "/etc/capsulebin/branding/logo.png"              # PNG or WebP
logo_dark = "/etc/capsulebin/branding/logo-dark.webp"   # optional, dark theme
favicon = "/etc/capsulebin/branding/favicon.png"        # PNG or ICO
# hide_name = true                                      # logo only, no name next to it
# tagline = "Encrypted pastes for the team"             # slogan on the create page, "" removes it
# source_url = "https://git.example.org/capsulebin"     # if you run a modified version

# [branding.de]                                         # another language than the default
# tagline = "Verschlüsselte Pastes fürs Team"           # CAPSULEBIN__BRANDING__DE__TAGLINE
```

- `name` replaces "CapsuleBin" in page titles, the header and the texts that name the service; at
  most 64 characters. It is not used by the CLI, the API or the logs.
- `tagline` replaces the built-in, translated slogan next to the logo on the create page in every
  language; `[branding.<lang>] tagline` replaces it in one language other than the default one.
  Without either, the built-in slogan; an empty string removes it. At most 80 characters.
- The logo is shown 1.5rem high and at most 12rem wide; a PNG at twice that height stays sharp. The
  dark logo follows the system preference and the theme toggle. `hide_name` and `logo_dark` need
  `logo`.
- Only raster files are accepted: SVG is refused. Each file at most 256 KiB, checked by extension
  and content. Relative paths are resolved against the working directory. In the container, mount
  the files read-only and use absolute paths, e.g. `-v ./branding:/etc/capsulebin/branding:ro`.
- The files are read at startup and served at `/branding/<hash>.<ext>` with long-lived caching;
  after replacing a file, restart the server (the URL changes with the content). `capsulebin check`
  reports a missing, oversized or wrongly formatted file.
- Every page says "Powered by CapsuleBin" in its footer, with "CapsuleBin" linked to the source
  code (AGPL-3.0, section 13), opened in a new tab. It cannot be switched off. If you run a
  modified version, set `source_url` to your source code (an absolute http(s) URL).

See [branding.md](branding.md) for the rules in detail.

## Health and logs

- `GET /health/live` — the process runs.
- `GET /health/ready` — configuration, database, schema and embedded assets (every page of every
  interface language) are usable. Use this for load balancer checks.
- `capsulebin healthcheck` probes `/health/ready` on the address from `server.bind` and exits
  non-zero unless it answers `200` — for container health checks, where there is no `curl`.
  `--live` probes `/health/live` instead; use that where an unhealthy container gets restarted,
  because a database that is briefly unavailable is a reason to wait, not to restart. `--address`
  overrides the address for a server started with `serve --bind`.
- Logs go to stderr. `CAPSULEBIN_LOG` sets the filter (default `info,tower_http=info,sqlx=warn`).
  The access log contains method, redacted path (`/api/v1/pastes/{id}`), status and latency —
  never paste ids, bodies, tokens or report text.
- Configuration errors go to stderr as well. Those about `server.public_url` name the key and the
  reason without repeating the value, and a file that is not valid TOML is reported as
  `line L, column C: <reason>` without the offending line — a share link pasted there by mistake
  does not end up in the service log.

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

The test suite upgrades a database written by 1.0 on every change: a released migration cannot
change, and capsules, burn tombstones, reports and moderation decisions from 1.0 stay usable.

#### A `public_url` whose path ends in a language code

An instance whose `server.public_url` has a path ending in an interface language code
(`https://example.org/team/de`, `…/team/en`) does not start. The same holds when the path only
ends in a code once its dot segments are resolved, as every client's URL parser does
(`…/team/de/.`, `…/team/x/../de`). `capsulebin --config … check` reports it, naming
`server.public_url`, so step 3 above fails before the service is started.

Share and management links already issued under such a base are ambiguous: the CLI reads the
language code in them as the language prefix of a page and drops it, so `get` and `delete` would
address the base without it (`https://example.org/team` instead of `https://example.org/team/de`).
Do not use these links with the CLI.

What to do:

1. Move the instance to a host of its own, served at `/` (`https://paste.example.org`), and set
   `server.public_url` to it. That is the setup the web interface needs: the server answers
   pages, `/assets/…` and `/api/…` at the root of its address only, the pages load their scripts
   and styles from `/assets/`, call `/api/`, and create share and management links directly under
   the origin. Below a path (`https://example.org/team/paste`) the web interface does not work,
   whatever the path ends in. Only the API and the CLI build on a base with a path — `put`
   appends `/api/v1/…`, `/p/<id>` and `/manage/<id>` to its `--server` — and only if the reverse
   proxy removes that path before it forwards the request; no test covers such a setup.
2. Links handed out under the old base keep pointing there. Whether and how they are mapped to
   the new base is up to the operator; CapsuleBin does not do it.

### Retention

The purge job runs at start and then daily; `capsulebin prune` applies it once on demand (for
example after lowering a threshold). It also removes closed abuse reports and moderation records
older than `abuse.report_retention`.

### Rotating creation tokens

Generate a new token with `capsulebin token`, add its hash, hand it out, then remove the old hash
and restart. Existing capsules are unaffected — the token only authorizes creation.

## Abuse handling runbook

Reports arrive through the report form (`/report`, `/de/report`). Only reports about capsules that
currently exist reach the queue.

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
