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

1. Write the configuration. The minimum for production:

   ```toml
   [server]
   bind = "127.0.0.1:8080"
   public_url = "https://paste.example.org"
   trusted_proxies = ["127.0.0.1/32"]   # the reverse proxy's own address

   [database]
   path = "/var/lib/capsulebin/capsulebin.db"

   [creation]
   mode = "token"                         # the default; tokens are issued in step 2

   [abuse]
   contact = "abuse@example.org"

   [legal.operator]                       # the imprint; see "Legal pages"
   file = "/etc/capsulebin/legal/operator.md"
   ```

2. Issue a creation token for each person or system that may create capsules (see
   [Creation tokens](#creation-tokens)):

   ```sh
   capsulebin --config /etc/capsulebin.toml token create --label "alice"
   ```

   It prints the token and an invitation link; hand out one of them. Tokens can be issued and
   revoked at any time later, while the server runs. Without any token the instance starts but
   refuses every create, and `check` and `serve` warn about it. Run the command as the user the
   server runs as (e.g. `sudo -u capsulebin …`): it opens the database, and creates it owner-only
   if it is missing.

3. Verify before opening a port, then start:

   ```sh
   capsulebin --config /etc/capsulebin.toml check
   capsulebin --config /etc/capsulebin.toml serve
   ```

### Creating capsules in the browser

The create page follows `creation.mode`, which the server puts into the page
([ADR-018](adr/ADR-018-creation-token-in-the-browser.md)):

- **`token`** (default): without a token the page shows no editor, only a notice that a creation
  token is needed, a token field and "Remember in this browser" (checked by default). Once a token
  is entered, the editor appears with "Forget" above it. The token is sent only as the bearer of
  the create request. A token the server refuses is removed from the browser at once; the person
  can enter another one and the same encrypted capsule is sent again, so nothing is lost.
- **`public`**: the page works without a token, but anyone can upload — only sensible behind
  another access control (VPN, SSO proxy).
- **`disabled`**: the page shows only a notice that the instance accepts no new capsules; existing
  links keep working.

The CLI takes the token from `CAPSULEBIN_TOKEN` or `--token-file`.

**Invitation link.** Instead of the 43-character token, hand out a link
`<public_url>/#token=<token>`. Opening it stores the token in that browser (`localStorage`,
`capsulebin.creation_token`) and shows "This browser can now create capsules"; the page removes
the fragment from the address bar first. If that browser already stores a different token, the
page asks whether to keep it or use the new one, and changes nothing until the user chooses. The
token stays in the fragment, which browsers never
send, so it reaches neither CapsuleBin nor the reverse proxy's log. The link works under every
language prefix (`/de/#token=…`); on a `public` or `disabled` instance it is removed and nothing
is stored. `capsulebin token create` always prints it (see [Creation tokens](#creation-tokens)),
plain `capsulebin token` when `server.public_url` is known.

Treat an invitation link like the token itself: whoever has it, or the browser profile that stored
it, can create capsules until the token is revoked (or its hash removed from the configuration).

### Creation tokens

In `token` mode a create needs a bearer token that is either an active token kept in the database
or one whose hash is listed in `creation.token_hashes`
([ADR-019](adr/ADR-019-creation-tokens-in-the-database.md)). Database tokens are the normal case:
they are managed with the operator CLI while the server runs, and every change applies from the
next create request, without a restart.

```sh
capsulebin --config /etc/capsulebin.toml token create --label "alice"   # issue
capsulebin --config /etc/capsulebin.toml token list                     # all, active and revoked
capsulebin --config /etc/capsulebin.toml token revoke <id>              # final
```

These commands validate the configuration like `serve` (so `server.public_url` must be set) and
open the database; give them the server's configuration.

- **`token create [--label <text>]`** stores only the token's domain-separated hash, the label,
  and the creation time, and prints:

  ```text
  token (store this in your client, it is not recoverable):
    <token>
  id: 01KAPS…
  label: alice
  invitation link (opens the create page and stores the token in the browser):
    https://paste.example.org/#token=<token>
  ```

  The token cannot be shown again. A label is optional, at most 64 characters and without control
  characters — a note for `token list`, e.g. whom the token is for. On a `public` or `disabled`
  instance the token is still created, with a `note:` on stderr that it has no effect until the
  mode is `token`.
- **`token list`** starts with `active tokens: N (configured hashes: M)`, then one line per token,
  oldest first: id, created, last used (the day only, or `never`), the state (`active`, or
  `revoked` with the time of the revocation) and label.
  `--json` prints a plain array of objects with `id`, `label`, `created_at`, `last_used_on` and
  `revoked_at` (Unix seconds or `null`). Neither shows a token or its hash.
- **`token revoke <id>`** takes the id from `token list`. The next create with that token gets
  `401`; a browser that stored it forgets it and asks for another one. Revoking is final — there
  is no command to undo it — and revoked tokens stay listed. Revoking an already revoked token
  says so and exits `0`; an unknown id exits `1`.

Per token the server records the day it last authorized a create (`last used`), nothing more: no
capsule is linked to the token that created it. Revoked entries are kept, so that a
[restore](#restore) can never bring a revoked token back.

`serve` (in its log) and `check` (as `warning: …` on stderr, still `ok:` and exit 0) warn when the
mode is `token` but no token exists at all (no configured hash, no active database token): the
server runs, but refuses every create until a token is issued. They also warn when active database
tokens exist while the mode is `public` or `disabled`, where they have no effect; tokens may be
prepared before switching the mode.

**Configured hashes.** `creation.token_hashes` keeps working alongside the database, for setups
configured entirely by file or environment. Plain `capsulebin token` (no subcommand) generates a
token and its entry; it stores nothing and needs no configuration. Changing the list needs a
restart; listing hashes is only allowed with `mode = "token"`.

```text
token (store this in your client, it is not recoverable):
  <token>

configuration entry:
  [creation]
  mode = "token"
  token_hashes = ["sha256:<hash>"]

invitation link (opens the create page and stores the token in the browser):
  https://paste.example.org/#token=<token>
```

The first lines are unchanged, so scripts that read the token from line 2 and the `sha256:` line
keep working. Plain `token` validates only `server.public_url`, by the rules `serve` applies; a
value that breaks one gives a `note:` on stderr naming the reason (never the value) and no link,
still with exit `0`. A `--config` that cannot be loaded is an error and prints no token. Without
`--config` and `CAPSULEBIN__SERVER__PUBLIC_URL` the output ends after the hash.

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

CapsuleBin creates a missing database file itself, exclusively and with mode `0600`, whatever the
umask; SQLite gives its `-wal` and `-shm` the same mode. Backups are written `0600` as well (see
[Backups](#backups)). `UMask=0077` is defence in depth for anything else the service writes.

Existing files are never changed. `serve` (in its log) and `check` (as `warning: …` on stderr,
still `ok:` and exit 0) warn about a database, `-wal` or `-shm` that grants any permission to
group or others, and about a database directory that group or others can write. Each warning names
the path and its octal mode; fix it with `chmod` if the access is not intended. The startup check
that the database directory is writable creates and removes a file with a random name
(`.capsulebin-write-probe-<hex>`) and never opens an entry that already exists.

### Container

Releases publish `ghcr.io/capsulebin/capsulebin:<version>` (also tagged `<major>.<minor>`,
`<major>` and `latest`) for `linux/amd64` and `linux/arm64`; how to verify its signature is in the
[README](../README.md#installation). The `Dockerfile` at the repository root builds the same image
locally: it builds the web client, embeds it into the binary and copies the result into a
distroless image without OpenSSL — no shell, no package manager, user `65532`. It is configured through
environment variables or a mounted configuration file and keeps the database on the `/data` volume.
The project's licence and the licence notices of the third-party code in the image are in
`/usr/share/doc/capsulebin/` (`LICENSE`, `THIRD_PARTY_LICENSES`).

```sh
docker pull ghcr.io/capsulebin/capsulebin:1.0.0   # or: docker build -t capsulebin .
docker tag ghcr.io/capsulebin/capsulebin:1.0.0 capsulebin
docker run -d --name capsulebin -p 127.0.0.1:8080:8080 -v capsulebin-data:/data \
  -e CAPSULEBIN__SERVER__PUBLIC_URL=https://paste.example.org \
  -e CAPSULEBIN__SERVER__TRUSTED_PROXIES='["172.16.0.0/12"]' \
  -e CAPSULEBIN__ABUSE__CONTACT=abuse@example.org \
  capsulebin
docker exec capsulebin capsulebin token create --label "alice"   # token + invitation link
```

With configured hashes instead of database tokens, generate one with
`docker run --rm capsulebin token` (with `-e CAPSULEBIN__SERVER__PUBLIC_URL=…` also the invitation
link) and pass `-e CAPSULEBIN__CREATION__TOKEN_HASHES='["sha256:…"]'`.

- The image sets `CAPSULEBIN__SERVER__BIND=0.0.0.0:8080` and
  `CAPSULEBIN__DATABASE__PATH=/data/capsulebin.db`. A configuration file can be mounted instead and
  passed as `--config /path serve`.
- Environment variables are meant for short, single-line values. Put long or multi-line texts,
  especially the [legal texts](#legal-pages), into a configuration file or a Markdown file referenced by `file`,
  mount it read-only and pass it at start (`--config /etc/capsulebin/capsulebin.toml serve`).
  An environment value is taken verbatim as a string (unless it reads as a boolean, an integer or
  an array): quotes and escape sequences such as `\n` are not interpreted, so Markdown passed this
  way does not come out as written.
- `trusted_proxies` must name the address the proxy connects *from* as the container sees it — with
  a published port that is usually the bridge gateway, not `127.0.0.1` (`docker network inspect
  bridge`, `Gateway`). CapsuleBin never logs client addresses, so take the proxy's source address
  from the Docker side and narrow the range to it.
- The image's health check runs `capsulebin healthcheck`, which probes `/health/ready` from inside
  the container (see [Health and logs](#health-and-logs)). It runs without `--config` and needs only
  `CAPSULEBIN__SERVER__BIND`, which the image sets and which overrides a `bind` in a mounted file,
  so it works for a container configured by a file as well. Podman drops `HEALTHCHECK` from images
  in its default OCI format; build with `podman build --format docker` to keep it.
- Logs go to stderr without colour codes, so `docker logs` shows plain lines.
- Operator commands run in the container, which has the server's configuration:
  `docker exec capsulebin capsulebin reports list`, `docker exec capsulebin capsulebin token list`.
  Backups go to the volume, e.g. `docker exec capsulebin capsulebin backup --output
  /data/backup-2026-11-18.db`; copy them off the host from there.
- Restore with the container stopped, from a one-off container on the same volume. `restore`
  validates the configuration like `serve`, so give it the same settings — the server's environment
  variables (at least `CAPSULEBIN__SERVER__PUBLIC_URL` and, if set,
  `CAPSULEBIN__CREATION__TOKEN_HASHES`) and the same mounted legal and branding files, or the
  mounted file with `--config` (see [Restore](#restore)):

  ```sh
  docker stop capsulebin
  docker run --rm -v capsulebin-data:/data \
    -e CAPSULEBIN__SERVER__PUBLIC_URL=https://paste.example.org \
    capsulebin restore --input /data/backup-2026-11-18.db --force
  # with a configuration file:
  # docker run --rm -v capsulebin-data:/data -v ./capsulebin.toml:/etc/capsulebin/capsulebin.toml:ro \
  #   capsulebin --config /etc/capsulebin/capsulebin.toml restore --input /data/backup-2026-11-18.db --force
  docker start capsulebin
  ```

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
limit_conn_zone $binary_remote_addr zone=capsulebin_conn:10m;   # in the http block

server {
    listen 443 ssl http2;
    server_name paste.example.org;
    add_header Strict-Transport-Security "max-age=63072000; includeSubDomains" always;
    client_max_body_size 16m;   # above pastes.max_plaintext_size × 4/3
    client_body_timeout 30s;    # between two reads of the body, not for the whole upload
    limit_conn capsulebin_conn 20;
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

**Slow and parallel uploads are the proxy's job.** CapsuleBin has no request or body timeout of its
own and no instance-wide budget for uploads in flight; what it limits is the size of each body (for
create `pastes.max_plaintext_size` × 4/3 plus 64 KiB, the base64 upload; fixed 4 KiB for describe,
retrieve and delete and 32 KiB for reports, `413 payload_too_large` above) and, if configured,
uploads per client (`rate_limits.create.max_concurrent`). Let the proxy end a body that stalls
(nginx `client_body_timeout`, Caddy `servers { timeouts { read_body … } }` in the global options)
and cap connections per client (nginx `limit_conn`). nginx buffers the whole body before passing it
on (the default `proxy_request_buffering on`), so a slow client ties up the proxy, not CapsuleBin.

**`trusted_proxies` must list the proxy, not the clients.** Too broad a range lets clients pick
their own address for rate limiting by sending `X-Forwarded-For`; empty behind a proxy makes every
client share the proxy's bucket.

CapsuleBin reads every `X-Forwarded-For` line, in order, as one list (a proxy may append its own
line instead of extending the client's) and walks it from the right, skipping trusted proxies. The
walk stops at the first entry that is not a bare IP address (`203.0.113.7:5678`, `unknown`, an
empty entry): the requests of such a client are counted under the last proxy reached, never under an
entry the client could have written. Let the proxy send bare addresses (nginx
`$proxy_add_x_forwarded_for`, Caddy's default). A listener on `[::]` sees IPv4 proxies as
`::ffff:a.b.c.d`; these addresses are compared as IPv4, so `trusted_proxies` takes the IPv4 range.

## Settings that matter

| Setting | Recommendation |
| --- | --- |
| `server.public_url` | The address users reach, without a trailing slash (refused at startup). It must be a valid http(s) URL; text that does not parse as one is refused at startup. User information (`https://user:pass@paste.example.org`, also only a user name or only a password, also percent-encoded) is refused at startup: it would reach every share link and every error's type URI. So is any whitespace or control character anywhere in the value, including a leading or trailing space or newline: the URL parser would silently drop some of them, while the value as written is what the server puts in front of every error's type URI. Use a host of its own with the instance at `/` (`https://paste.example.org`): the web interface loads `/assets/…`, calls `/api/…` and creates links directly under the origin, so it does not work below a path. A query or fragment, also an empty one (`https://example.org/team?`, `…#`), is refused at startup: paths are appended to the URL as text. Its path must not end in an interface language code (`https://example.org/team/de` is refused at startup, as is a path that only ends in one once dot segments are resolved, `…/team/de/.`): a link below it could not be told apart from the `/de/` language prefix. |
| `creation.mode` | `token` (default), with tokens issued by `capsulebin token create` (see [Creation tokens](#creation-tokens)). `public` only behind another access control. |
| `pastes.default_expiry` / `maximum_expiry` / `allow_permanent` | Short defaults keep the instance small; permanent capsules are off by default. |
| `rate_limits.*` | All off by default except `invalid_capability_per_paste` (wrong passwords per capsule, 10/min burst 10), which does not depend on `trusted_proxies`. Once `trusted_proxies` is right, a reasonable start: `create` 5/min burst 2, `bytes_per_day = "100MiB"`, `max_concurrent = 2`; `descriptor` 60/min burst 10; `retrieve` 30/min burst 5; `delete` 10/min burst 3; `report` 5/hour burst 2; `invalid_capability` 10/min burst 10. |
| `purge.*` | Leave enabled. Never-retrieved capsules go after 30 days, inactive ones after 90. |
| `abuse.contact`, `abuse.report_retention` | Set a contact; keep closed reports no longer than needed (default 90 days). |
| `abuse.max_open_reports_per_paste`, `abuse.max_open_reports` | Ceilings on open reports (status open or reviewing): 10 per capsule, 1000 on the instance by default; integers of at least 1. A report above a ceiling is not stored, and the reporter is told nothing different (see [Abuse handling runbook](#abuse-handling-runbook)). To stop taking reports, set `abuse.enabled = false` — `0` is refused. |
| `legal.*` | A public instance in the EU usually needs an imprint and a privacy notice. Set `[legal.operator]`; see [Legal pages](#legal-pages). |
| `branding.*` | Optional: name, tagline, logo, favicon and theme colours of the instance; `source_url` if you run a modified version. See [Branding](#branding). |
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
  the same rule.
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
- `[branding.colors.light]` and `[branding.colors.dark]` set the theme colours (the 18 colour
  tokens, `#rgb`/`#rrggbb`), served as a generated stylesheet at `/branding/<hash>.css`. A colour
  pair below the built-in contrast thresholds is a warning — printed by `check` (still `ok:`) and
  logged by `serve` — never an error.

See [branding.md](branding.md) for the rules in detail.

## Health and logs

- `GET /health/live` — the process runs.
- `GET /health/ready` — configuration, database, schema and embedded assets (every page of every
  interface language) are usable. Use this for load balancer checks.
- `capsulebin healthcheck` probes `/health/ready` on the address from `server.bind` and exits
  non-zero unless it answers `200` — for container health checks, where there is no `curl`.
  `--live` probes `/health/live` instead; use that where an unhealthy container gets restarted,
  because a database that is briefly unavailable is a reason to wait, not to restart. `--address`
  overrides the address for a server started with `serve --bind`; no configuration is read then.
  Otherwise the configuration (`--config` file and environment) is loaded only for `server.bind`:
  the command validates nothing else (no `public_url` needed), never writes to the database
  directory and opens neither the database nor a port.
- Logs go to stderr. `CAPSULEBIN_LOG` sets the filter (default `info,tower_http=info,sqlx=warn`).
  The access log contains method, path, status and latency — never paste ids, bodies, tokens or
  report text. The path is never the one the client sent: it is the template of the matched route
  (`/api/v1/pastes/{id}/retrieve`, `/de/p/{id}`, `/en/p/{id}` for the redirect from the default
  language's prefix, `/health/ready`, a configured `/branding/<hash>.<ext>`), the path of a static
  file the server actually served (`/assets/…`, `/favicon.svg`), or `{unmatched}` for everything
  that answered the generic `404`. A `429` or `413` on a route logs that route's template.
- The rate limiter keeps its buckets in memory, at most 100 000 client buckets and 100 000 capsule
  buckets (password attempts); this is fixed, not a setting. A table at its ceiling logs a warning
  at most once a minute (`rate limiter table full`, with `table` and `ceiling`, never an address or
  id). For the client table it means a flood of addresses: each new address pushes out the least
  recently used one, which then starts with a fresh budget. For the capsule table it means every
  retrieve that proves the link on a capsule the table does not yet track, with a correct password
  or a wrong one (or none: the server cannot tell), shares one overflow budget of
  `invalid_capability_per_paste` (10 a minute, burst 10 at the default; none when that limit is
  off) until buckets refill (about a minute at the default). Once it is used up, such retrieves get
  `429` with `Retry-After: 6`, the refill time of one attempt, so first retrieves of untracked
  capsules can be throttled while the table stays full; capsules already tracked keep their own
  budget. Correct and wrong passwords are answered alike, so a full table never tells which
  password is right.
- Configuration errors go to stderr as well. Those about `server.public_url` name the key and the
  reason (e.g. `must not contain user information`,
  `must not contain whitespace or control characters`) without repeating the value, and a file
  that is not valid TOML is reported as `line L, column C: <reason>` without the offending line —
  a share link pasted there by mistake does not end up in the service log.

## Routine tasks

### Backups

```sh
capsulebin --config /etc/capsulebin.toml backup --output /backups/capsulebin-$(date +%F).db
```

Safe while serving. The file is consistent, compacted and `0600`. It is written and verified inside
a new private directory (`.<name>.backup-<random hex>`, mode `0700`) next to the output, then
renamed to the output name; the directory is removed afterwards, also on failure. An existing
entry at the output name — a file, a directory or a symlink, dangling or not — is refused, checked
before the snapshot and again right before the rename. The rename never follows a symlink; only an
entry created in the output directory between that last check and the rename would be replaced,
which needs write access to that directory (and whoever has it could replace the backup anyway).
The rename stays within the output's directory, so any filesystem works as a destination. Store
backups as carefully as the database: they contain ciphertext, verifiers and abuse reports with
contact details. Rotate them — an old backup is where deleted capsules survive the longest. Never
copy the live database file directly.

### Restore

```sh
systemctl stop capsulebin
capsulebin --config /etc/capsulebin.toml restore --input /backups/capsulebin-2026-11-17.db --dry-run
capsulebin --config /etc/capsulebin.toml restore --input /backups/capsulebin-2026-11-17.db --force
systemctl start capsulebin
```

The server must be stopped. `restore` never opens the input file with SQLite and never touches it:

1. It refuses if a name it would write is taken: without `--force` an existing database, with
   `--force` `<file>.pre-restore-<timestamp>` or its `-wal`/`-shm`.
2. It copies only the backup's main file (never a `-wal` or `-shm` next to it) into a new private
   directory `.<file>.restore-<random hex>` (`0700`, copy `0600`) next to the database, so the
   database directory needs room for one more copy.
3. It verifies integrity, CapsuleBin schema and that the schema is not newer than the binary on
   that copy, applies pending migrations to it like `serve` would, and checkpoints it so it is one
   self-contained file.
4. If a database is at the configured path, it opens that one read-only (writing nothing to it or
   its `-wal`/`-shm`) and retires in the copy every id it holds, as a capsule or as an already
   retired id, that the backup does not hold as a capsule; then it checkpoints the copy again.
   Paste ids are never used twice: a capsule created after the backup still has its links in
   circulation, and freeing its id would let a link holder put different content behind them.
   A capsule both databases hold keeps the later of the two states: one the current database holds
   burned (consumed), deleted (by its owner or by moderation) or disabled while the backup holds it
   in an earlier state takes the current database's state, moderation reason and time, and loses its
   ciphertext wherever the current database has none. A restore therefore never makes a burned or
   deleted capsule readable again and never lifts a moderation made after the backup. It only moves
   a capsule forward: one disabled in the backup and re-enabled (`moderation restore`) after it
   stays disabled; run `moderation restore` again if that is still wanted.
   In the same step it carries the current database's [creation tokens](#creation-tokens) into
   the copy, matched by their hash: a token the backup lacks is added (with its id, label, dates
   and revocation), a token revoked in the current database stays revoked even if the backup holds
   it active, and the later day of last use is kept. A restore therefore never brings back a
   revoked token and never loses one issued after the backup; tokens only in the backup stay as
   they are.
5. Only then, with `--force`, the current database and its `-wal`/`-shm` move to
   `<file>.pre-restore-<timestamp>` (`-wal`, `-shm`), and the copy is renamed into place.

`restore` prints how many ids of the previous database it retired, how many capsules kept the
previous database's burned, deleted or disabled state, and how many creation tokens, revocations and
last-use days it carried over. If the current database cannot be read (it is
corrupt, or it has a non-empty `-wal` without its `-shm`, which reading would create), the restore
still installs the backup and prints
`warning: the ids, capsule states and creation tokens of the replaced database could not be carried
over (…)` on stderr, with exit code 0: those ids can then be used again, capsules it burned,
deleted or disabled after the backup are available again (delete or disable them again with
`moderation` if you know their ids), its tokens the backup lacks are lost,
and tokens it revoked are active again if the backup holds them unrevoked — check `token list` and
revoke them again. Only a failure to attach the current database or to read it in a first
read-only probe gives this warning. A current database from before creation tokens existed simply
has none to carry. A failure while writing the ids, states or tokens into the copy (the disk full, for
example), or a read failure that shows up only then, stops the restore before the cutover like any
other step.

A failure at any step undoes the moves made so far and removes the private directory: the
configured path holds either the old database, unchanged with its `-wal`/`-shm`, or the restored
one, never neither. If moving a file back fails as well, the error lists what to move where by hand.
Delete `<file>.pre-restore-<timestamp>` once the restored instance is confirmed working.

`--dry-run` runs steps 1 to 4 on a private copy, prints the backup's contents, the schema version
the migration reached, how many ids would be retired, how many capsules would keep their burned,
deleted or disabled state, and how many creation tokens, revocations and last-use days would be
carried over (with the same warning if the current
database cannot be read), then removes the copy; nothing else is created or changed. Without
`--force` it also says when the restore would need `--force`.

Restore only backups you made yourself: SQLite treats a database as trusted input. Verifying and
migrating only a private copy of the main file keeps a `-wal` next to the backup from reaching
SQLite, but does not make an untrusted main file safe.

**Interrupted restore.** If the restore process is killed or the machine loses power during the
cutover, the database file may be missing while `<file>.pre-restore-*` or a
`.<file>.restore-*` directory remains. Then `serve`, `check` and every other command that opens the
database refuse to start and name those entries instead of creating an empty database. Run the
restore again (it works next to an older `<file>.pre-restore-*` and leaves it alone), or move the
old database (and its `-wal`/`-shm`) back to the configured path. Without a database file and
without such entries, the first start creates a new database as usual; next to an existing
database these entries block nothing. A `.<file>.restore-*` or `.<name>.backup-*` directory left by
a killed run is not removed automatically; delete it once no restore or backup is running.

### Upgrades

1. `capsulebin backup --output …` with the running version.
2. Stop the service, replace the binary.
3. `capsulebin --config … check` — applies pending migrations and verifies schema and assets. A
   binary older than the database refuses to run rather than writing rows the schema cannot hold.
4. Start the service and check `/health/ready`.

Migrations only move forward. To roll back, restore the backup from step 1 with the old binary.

The test suite checks on every change that a database at an older schema migrates and stays
usable, and that no existing migration is edited.

### Retention

The purge job runs at start and then daily; `capsulebin prune` applies it once on demand (for
example after lowering a threshold). It also removes closed abuse reports and moderation records
older than `abuse.report_retention`; with `purge.enabled = false` capsules are no longer
purged on schedule, but that report retention still runs daily.

A purged capsule's id stays in the table `retired_ids`, alone and permanently, so it can never be
created again ([ADR-011](adr/ADR-011-idempotent-create.md)); no rule, setting or command removes it.
Reports about it keep showing the capsule as `gone`, and `moderation` answers "no paste … (never existed,
or already purged)". The table grows by about 40 bytes per purged capsule.

### Rotating creation tokens

For a database token: issue a new one with `capsulebin token create`, hand it out (or its
invitation link), then `capsulebin token revoke <old id>`. No restart is needed; both take effect
from the next create request, also while the server runs. `token list` shows from `last used`
whether a token is still in use.

For a configured hash: generate a new token with `capsulebin token`, add its hash, hand it out,
then remove the old hash and restart — or move to a database token and remove the hash.

Existing capsules are unaffected — the token only authorizes creation. A browser that stored the
old token gets `401` on its next create; the page forgets the token and asks for the new one (or
open a new invitation link there).

## Abuse handling runbook

Reports arrive through the report form (`/report`, `/de/report`). Only reports about capsules that
currently exist reach the queue, and only while the queue is below its ceilings: a capsule takes at
most `abuse.max_open_reports_per_paste` (default 10) open reports, the instance at most
`abuse.max_open_reports` (default 1000). Open means `open` or `reviewing`; closing a report frees
its place. A report above a ceiling is dropped, and the reporter gets the same answer as for a
stored one.

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

`--reason` takes `illegal_content`, `malware`, `phishing`, `spam`, `personal_data`, `copyright` or
`other`. `reports list` takes `--status` (default `open`), `--limit` (default 100) and `--json`.

- `reports list` starts with a line like `open reports: 3 of 1000 (at most 10 per capsule)`; when
  the instance ceiling is reached it adds `the limit is reached: new reports are not stored until
  open ones are closed`. Whenever a report is dropped at either
  ceiling, the server log says `report not stored: open report limit reached` — at most once a
  minute, without the capsule or the report. Work through the queue and
  close what is done (`duplicate` for repeats about the same capsule); if the queue is full of junk,
  close it as `invalid` and consider `rate_limits.report`. Raise a ceiling only if genuine reports
  are being lost: the ceilings are what keeps anyone who knows one id from growing the queue and the
  database without bound. `reports list --json` stays a plain array without this line.
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
| A creation token or invitation link leaked | `capsulebin token revoke <id>` (effective at once, no restart); for a configured hash, remove it and restart. Consider `creation.mode = "disabled"` while investigating — existing capsules stay readable. |
| Flood of uploads | Enable or tighten `rate_limits.create` (rate, `bytes_per_day`, `max_concurrent`); `creation.mode = "disabled"` stops all creation. |
| Flood of reports | The ceilings keep the queue bounded; `reports list` shows how full it is. Enable `rate_limits.report` (e.g. 5/hour burst 2), close junk as `invalid`; `abuse.enabled = false` stops reports while moderation keeps working. |
| Database or backup stolen | Content stays protected without the links. Reports in it contain contact data, and creation token labels may name people — treat it as a personal-data breach. Creation tokens are stored only as hashes; rotate them anyway. |
| Server compromised | Assume served JavaScript may have been modified: web users of that period may have exposed keys. Rebuild from a trusted binary, rotate tokens, inform users. CLI users were not exposed through served code. |
| Disk full | SQLite refuses writes; reads continue. Free space, run `prune`, check `pastes.max_plaintext_size` and the byte quota. |
| `/health/ready` answers `503` with `database: unavailable`, the log says `schema is at version … but this build expects …` | The binary and the database disagree. Run `check` with the matching binary; never edit `_sqlx_migrations` by hand. |
