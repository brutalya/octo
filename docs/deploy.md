# Deploy

## Options

| Option | For | How |
|---|---|---|
| Desktop app | PC | Installer from the GitHub Releases page (Windows, macOS, Linux). |
| From source | development | `scripts/dev.sh`, or `cd web && npm ci && npm run build` then `cargo run --release -p octo-server`. |
| Docker relay | public/own relay | `deploy/relay/docker-compose.yml`, below. |
| Docker node | own server node | `deploy/node/docker-compose.yml`, below. |
| systemd | no Docker | `deploy/systemd/` (units, Caddyfile example); binary at `/opt/octo/bin/octo-server`, user and paths adjustable. |

Images (built by `.github/workflows/docker.yml` on push to `main` and `v*` tags, linux/amd64 + arm64):
`ghcr.io/<owner>/<repo>-relay` and `ghcr.io/<owner>/<repo>-node`, tags `main`, `sha-<short>`, `X.Y.Z`, `X.Y`.

## Relay (behind Nginx Proxy Manager)

The relay serves only `/relay` (WebSocket) and `/healthz` on port 7443 and sees no plaintext.

1. Log in to GHCR (the repo is private; use a personal access token with `read:packages`):
   `echo "$GHCR_PAT" | docker login ghcr.io -u <github-user> --password-stdin`
2. Copy `deploy/relay/docker-compose.yml` to the server, replace `OWNER/REPO` in `image:` with the lowercase repository path.
3. Find the NPM network (`docker inspect <npm-container> --format '{{json .NetworkSettings.Networks}}'`); default is `npm_default`. If different: `export NPM_NETWORK=<name>` or put it in `.env` next to the compose file.
4. `docker compose up -d`
5. In NPM add a Proxy Host: domain `relay.example.com`, scheme `http`, forward host `octo-relay`, port `7443`, Websockets Support ON. SSL tab: request a Let's Encrypt certificate, Force SSL.
6. Cloudflare: proxied (orange cloud) works, WebSockets are supported.
7. Firewall: only 80 and 443 are needed. 7443 is not published by default. Without a shared network, uncomment `ports: "7443:7443"` and forward NPM to the Docker host address and port 7443 (then block 7443 externally).

Verify: `curl https://relay.example.com/healthz` returns `ok`.

Client setup: in Octo settings set the relay URL to `wss://relay.example.com/relay` on every device of the network.

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

`--addr` (API, default `127.0.0.1:7878`; non-loopback needs `--token` or `--token-file`), `--token-file <path>` (created with a random token, mode 0600, if missing), `--relay-addr`, `--relay-only`, `--data-dir`, `--web-dir`, `--healthcheck <addr>` (probe `/healthz`, exit code 0/1).

## Troubleshooting

- 502 from NPM: NPM cannot reach the container. Check both are on the same Docker network (`docker network inspect <network>`), the forward host is the container name, and the container is healthy (`docker compose ps`, `docker compose logs`).
- `pull access denied`: GHCR login missing or the token lacks `read:packages`.
- Client cannot connect: the relay URL must be `wss://.../relay` and Websockets Support must be ON in NPM.
