# Deploy

## Options

| Option | For | How |
|---|---|---|
| Desktop app | PC, phone | Latest release on the GitHub Releases page (Windows, Linux, Android APK). |
| From source | development | `scripts/dev.sh`, or `cd web && npm ci && npm run build` then `cargo run --release -p octo-server`. |
| Docker relay | public/own relay | `deploy/relay/docker-compose.yml`, below. |
| Docker node | own server node | `deploy/node/docker-compose.yml`, below. |
| systemd | no Docker | `deploy/systemd/` (units, Caddyfile example); binary at `/opt/octo/bin/octo-server`, user and paths adjustable. |

Images (built by `.github/workflows/docker.yml` on merges to `main` that change code or deploy files, and for every release tag; linux/amd64):
`ghcr.io/<owner>/<repo>-relay` and `ghcr.io/<owner>/<repo>-node`, tags `main`, `sha-<short>`, `X.Y.Z`, `X.Y`.

## Relay (behind Nginx Proxy Manager)

The relay serves only `/relay` (WebSocket) and `/healthz` on port 7443 and sees no plaintext.

1. Log in to GHCR (packages are private unless made public in the package settings; use a personal access token with `read:packages`):
   `echo "$GHCR_PAT" | docker login ghcr.io -u <github-user> --password-stdin`
2. Copy `deploy/relay/docker-compose.yml` to the server, replace `OWNER/REPO` in `image:` with the lowercase repository path.
3. Find the NPM network (`docker inspect <npm-container> --format '{{json .NetworkSettings.Networks}}'`); default is `npm_default`. If different: `export NPM_NETWORK=<name>` or put it in `.env` next to the compose file.
4. `docker compose up -d`
5. In NPM add a Proxy Host: domain `relay.example.com`, scheme `http`, forward host `octo-relay`, port `7443`, Websockets Support ON. SSL tab: request a Let's Encrypt certificate, Force SSL.
6. Cloudflare: proxied (orange cloud) works, WebSockets are supported.
7. Firewall: only 80 and 443 are needed. 7443 is not published by default. Without a shared network, uncomment `ports: "7443:7443"` and forward NPM to the Docker host address and port 7443 (then block 7443 externally).

Verify: `curl https://relay.example.com/healthz` returns `ok`.

The compose file runs a **private** relay: it admits only nodes that know its secret, created on first start in the `relay-data` volume. Read it with `docker compose exec octo-relay cat /data/relay-secret`.

Client setup: in Octo settings → Devices on the first device, enter `relay.example.com` as the relay address and the secret as the relay key (stored as `wss://relay.example.com/relay?key=<secret>`), then press Check. Devices that join with a pairing code get the relay URL, key included, from the code. The key is sent in the `Authorization` header, not in the request path, so proxy access logs do not contain it. Changing the secret (delete the file, restart) requires the new URL on every device.

For an open relay (any Octo node may use it) remove `command` and the volume from the compose file.

Update: `docker compose pull && docker compose up -d`.

## Node (web UI + API + Claude Code)

1. Same GHCR login, then copy `deploy/node/docker-compose.yml`, set `OWNER/REPO` and `NPM_NETWORK` as above, `docker compose up -d`.
2. The token is created on first start: `docker compose exec octo-node cat /data/token`. The API requires it (`Authorization: Bearer <token>`).
3. NPM Proxy Host: forward host `octo-node`, port `7878`, Websockets ON, Let's Encrypt SSL.
4. Log in to Claude Code once; the login persists in the `/home/octo/.claude` volume: `docker compose exec -it octo-node claude`.
5. Antigravity CLI is not bundled. To add it, build a derived image (`FROM` the node image, install `agy` as user `octo`, mount its state dir as a volume) and pass `--agy-bin` / `--agy-dir` if the paths differ from the defaults.

Volumes: `/data` (database, keys, token), `/home/octo/.claude` (CLI login). Back up `/data`.

Update: `docker compose pull && docker compose up -d`.

## Flags

`--addr` (API, default `127.0.0.1:7878`; non-loopback needs `--token` or `--token-file`), `--token-file <path>` (created with a random token, mode 0600, if missing; **preferred**: `--token <secret>` is visible in the process list), `--token <secret>` (must not be empty), `--relay-addr`, `--relay-only`, `--relay-secret-file <path>` (private relay: clients need `?key=<secret>` in the relay URL; created with a random secret, mode 0600, if missing), `--data-dir`, `--web-dir <dir>` (default `./web/dist` if it exists; an explicit path that is not a directory is a startup error), `--healthcheck <addr>` (probe `/healthz`, exit code 0/1).

An empty `--token` or an empty token file is a startup error. Configuration is validated before any port is bound. SIGTERM and Ctrl-C stop accepting connections, end running agent processes and exit.

Without a token the API only answers requests whose `Host` header is `127.0.0.1`, `localhost`, `[::1]` or the host of `--addr` (protection against DNS rebinding); with a token the header is not checked.

The relay listener (`--relay-addr`) only forwards end-to-end encrypted frames between nodes of one network id and sees no plaintext. Without `--relay-secret-file` it is open to any Octo node; with it, upgrades without `Authorization: Bearer <secret>` get 401 before any handshake. `/healthz` stays open.

## Troubleshooting

- 502 from NPM: NPM cannot reach the container. Check both are on the same Docker network (`docker network inspect <network>`), the forward host is the container name, and the container is healthy (`docker compose ps`, `docker compose logs`).
- `pull access denied`: GHCR login missing or the token lacks `read:packages`.
- Client cannot connect: the relay URL must be `wss://.../relay` (plus `?key=<secret>` for a private relay) and Websockets Support must be ON in NPM. A wrong or missing key shows as 401 in the relay's proxy log.
