# Coppice MAM gateway

`mam-gateway` is a deliberately narrow, standalone Rust sidecar. It owns the
MAM session and tracker download URL, translates the allowlisted Torznab search
surface, and submits gateway-fetched torrent bytes to qBittorrent over loopback.
Coppice never receives a MAM cookie, tracker URL, magnet, or qBittorrent URL.
This is clean-room protocol behavior; the service does not depend on Prowlarr,
Jackett, Lodestarr, or another indexer manager.

## Contract

| Route                                     | Auth                                    | Behavior                                                                                                                                                      |
| ----------------------------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /torznab/api?t=caps`                 | Bearer token or legacy Torznab `apikey` | Capabilities for `q`/`cat` book search. Coppice uses the bearer header, not a URL secret.                                                                     |
| `GET /torznab/api?t=search&q=...&cat=...` | Bearer token or legacy Torznab `apikey` | Searches MAM and returns normalized rows whose `guid` is a short-lived opaque result id. The link is a URN, never a tracker URL.                              |
| `GET /v1/results/{result_id}`             | Bearer token or `X-Api-Key`             | Returns the safe normalized metadata for one unexpired result.                                                                                                |
| `POST /v1/grabs`                          | Bearer token or `X-Api-Key`             | JSON `{ "result_id": "...", "idempotency_key": "..." }`. The optional `Idempotency-Key` header must exactly match the body field.                             |
| `GET /v1/grabs/{grab_id}`                 | Bearer token or `X-Api-Key`             | Polls qBittorrent by a gateway-owned tag and returns `status` plus bounded progress; `COMPLETED` includes a safe relative handoff only after payload hashing. |
| `GET /v1/health`                          | None (liveness)                         | Unauthenticated process liveness only; it is not an authentication or upstream-health proof.                                                                  |

`result_id` and `grab_id` are 32 hexadecimal characters generated with UUID v4.
Results and grab records expire in memory (15 minutes by default); a restart
invalidates outstanding handles. A repeated idempotency key returns the same
`grabId` and never uploads a second torrent. Unknown JSON fields, query
parameters, categories, or direct URL/magnet inputs are rejected.

## Configuration

Secrets prefer a file over an environment variable and are never included in
logs or error bodies:

| Variable                                        | Default                      | Notes                                                               |
| ----------------------------------------------- | ---------------------------- | ------------------------------------------------------------------- |
| `MAM_GATEWAY_BIND`                              | `0.0.0.0:8088`               | Listen address; bind privately in deployment.                       |
| `MAM_GATEWAY_TOKEN` / `_FILE`                   | required                     | Coppice-facing bearer/API token.                                    |
| `MAM_COOKIE` / `_FILE`                          | required                     | Gateway-only MAM session cookie.                                    |
| `MAM_BASE_URL`                                  | `https://t.myanonamouse.net` | Operator setting; production MAM hosts are allowlisted.             |
| `MAM_SEARCH_PATH`                               | `/tor/js/loadSearchJSON.php` | Fixed path, never client supplied.                                  |
| `QBIT_URL`                                      | `http://127.0.0.1:8080`      | Loopback-only and HTTP-only by validation.                          |
| `QBIT_USERNAME` / `QBIT_PASSWORD` (and `_FILE`) | unset                        | Optional qBittorrent WebUI credentials; both are required together. |
| `MAM_GATEWAY_TIMEOUT_SECS`                      | `15`                         | Every upstream request timeout.                                     |
| `MAM_RESULT_TTL_SECS`                           | `900`                        | Opaque result/grab lifetime (60–86400).                             |
| `MAM_MAX_TRACKER_BODY_BYTES`                    | `2097152`                    | Bounded MAM JSON response.                                          |
| `MAM_MAX_TORRENT_BYTES`                         | `8388608`                    | Bounded gateway-fetched torrent artifact.                           |
| `QBIT_MAX_BODY_BYTES`                           | `1048576`                    | Bounded qBittorrent API response.                                   |
| `MAM_GATEWAY_HANDOFF_ROOT`                      | `/downloads`                 | Absolute root read by the gateway after qBittorrent completion.     |
| `MAM_GATEWAY_MAX_HANDOFF_BYTES`                 | `17179869184`                | Streaming SHA-256 handoff size limit.                               |

The HTTP clients disable redirects. A redirect, body-limit breach, timeout,
upstream 401/403, invalid source JSON, unsafe source host, or qBittorrent
failure becomes a stable redacted error code. The tracker URL is accepted only
when it is an operator-allowlisted MAM host; there is no client-supplied fetch
URL and no arbitrary qBittorrent endpoint.

## Container boundary

`deploy/compose.yml` runs Gluetun, `mam-gateway`, and qBittorrent with the
required topology:

1. Gluetun joins the external, internal-only `coppice-control` network.
2. The gateway and qBittorrent use `network_mode: service:gluetun`.
3. qBittorrent listens on `127.0.0.1:8080`; the gateway talks to that loopback.
4. Only Gluetun's `8088` is admitted on the private control network. No service
   publishes tracker, qBittorrent, or gateway ports on the host.
5. Coppice calls `http://vpn-gateway:8088` over `coppice-control`.
6. The completed-download volume is mounted read-only into the gateway at
   `/downloads` and into Coppice at its configured handoff root; qBittorrent is
   the sole writer.

The compose service runs the gateway as qBittorrent's read-compatible UID
(`1000:1000`) while keeping the handoff mount read-only.

Create the network before starting the stack:

```sh
docker network create --driver bridge --internal coppice-control
cp deploy/.env.example deploy/.env
mkdir -p deploy/secrets deploy/qbittorrent-config deploy/downloads
# Write the gateway token and MAM cookie into deploy/secrets/ files.
docker compose --env-file deploy/.env -f deploy/compose.yml up -d
```

Verify qBittorrent's generated config retains `WebUI\Address=127.0.0.1` and
`WebUI\Port=8080`. For a production deployment, enable qBittorrent WebUI
authentication and set the matching gateway `QBIT_*` secret files; the sample
keeps local-only authentication disabled because qBittorrent is loopback-bound.
The Gluetun kill switch and VPN health state must fail closed.

## Fixtures and tests

`tests/fixtures` contains a fake MAM JSON response, a cookie-gated torrent
artifact, and qBittorrent info/add responses. `tests/protocol.rs` exercises the
observable caps/search/grab/poll contract, verifies the fixture cookie is seen
only upstream, and proves duplicate grab submission performs one qBittorrent
add. Fixtures are synthetic and contain no real credentials.
