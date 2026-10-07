# beans-relay

The relay stores public keys and ciphertext and forwards blobs between an identity's Devices. The [Relay doc](../../web/content/docs/relay.mdx) covers running it and [docs/architecture/relay.md](../../docs/architecture/relay.md) the protocol and storage.

Every flag has an environment variable, listed by `beans-relay --help`.

Account sync uses protocol 4. Roster PUTs require an effective floor of at least 4 even with a lower generic minimum. Individual blob DELETE protects roster and policy rows in the atomic SQLite/Postgres operation for every admitted client; protected ids return `404` without changing sequence or usage. See [protocol compatibility](../../docs/architecture/avatars.md#protocol-compatibility-and-rollout).

## Public catalogs

Set `BEANS_RELAY_CATALOG_DIR` (or `--catalog-dir`) to an operator-managed directory containing `models/v1.json` and `marketplace/v1.json`. The relay serves these plaintext files at `GET /models/v1.json` and `GET /marketplace/v1.json` without auth or a `Beans-Protocol` header. Each file must be valid JSON no larger than 1 MiB; missing files answer 404, malformed or oversized files answer 500. The relay loads files at startup, so replace them and restart the server to publish changes without a client release. Mount the same directory on every replica; do not place credentials or account data there. Clients use their bundled catalogs when feeds are unavailable.

## Deploy on Railway

1. Create a service from this repository and leave **Root Directory** empty: the [`Dockerfile`](Dockerfile) builds with the whole Cargo workspace as context. In the service's **Settings**, set **Healthcheck Path** to `/v1/health`, and **Watch Paths** to `/crates/relay/**`, `/Cargo.toml`, and `/Cargo.lock` so that pushes elsewhere in the repo leave the relay running.
2. Set the [variables every deploy needs](#variables-every-deploy-needs).
3. Pick storage: [SQLite on a volume](#sqlite-on-a-volume) or [Postgres and a bucket](#postgres-and-a-bucket).
4. Generate a domain under **Settings › Networking**.
5. Point your first Device at `https://<domain>`: **Settings › Advanced › Relay URL** in the app, or `BEANS_RELAY_URL` for the CLI. Devices you pair afterwards learn the URL from the pairing code.

Once it is up, `https://<domain>` reads "Beans Relay is running...", `curl https://<domain>/v1/health` answers `{"ok":true,"service":"beans-relay"}`, and the relay's first log line names its address, database, file store, and push services.

### Variables every deploy needs

| Variable | Value |
| --- | --- |
| `RAILWAY_DOCKERFILE_PATH` | `crates/relay/Dockerfile`. Railway builds the service from this file. |
| `BEANS_RELAY_SECRET` | The output of `openssl rand -hex 32`. It signs bearer tokens; unset, it changes on every boot and invalidates every Device's token. |
| `BEANS_RELAY_TRUST_PROXY` | `true`. The relay takes the client address from the `X-Forwarded-For` header that Railway's edge sets; otherwise every request comes from the proxy and all clients share one rate limit. |
| `RAILWAY_DEPLOYMENT_DRAINING_SECONDS` | `10`. How long Railway waits between the SIGTERM that stops the old relay and a SIGKILL. Railway's default is 0, which kills the relay at once: requests in progress are cut off, for the Devices to send again; pushes it took but had not yet handed to APNs or FCM are lost; and with Postgres, its sockets keep counting as online until its last heartbeat is 150 s old. Given time, it finishes its requests, delivers those pushes (up to 5 s), takes its sockets out of presence, and exits. With Postgres the new relay is already serving by then, so the wait costs no downtime. |

Leave `PORT` and `BEANS_RELAY_BIND` unset. Railway sets `PORT`, the image listens on `[::]:$PORT`, and the healthcheck calls that port. `BEANS_RELAY_BIND` takes precedence over `PORT`.

### SQLite on a volume

Add a volume mounted at `/data`; no variable is needed. The image runs in `/data`, so the relay keeps `/data/beans-relay.db` and `/data/beans-relay.files`.

Railway runs one deployment at a time on a volume: a deploy stops the old relay before the new one starts, and Devices reconnect a few seconds later.

### Postgres and a bucket

With Postgres, Railway starts the new deployment before it stops the old one, and the service can run more than one replica. Add Railway's Postgres to the project and give the relay no volume.

| Variable | Value |
| --- | --- |
| `BEANS_RELAY_DB` | `${{Postgres.DATABASE_URL}}?sslmode=disable` |
| `BEANS_RELAY_S3_BUCKET` | The bucket's name. |
| `BEANS_RELAY_S3_ENDPOINT` | `https://<account id>.r2.cloudflarestorage.com`, `https://s3.us-east-1.amazonaws.com`, … |
| `BEANS_RELAY_S3_ACCESS_KEY` | Falls back to `AWS_ACCESS_KEY_ID`. |
| `BEANS_RELAY_S3_SECRET_KEY` | Falls back to `AWS_SECRET_ACCESS_KEY`. |
| `BEANS_RELAY_S3_REGION` | The SigV4 region; `auto` (R2) by default. |
| `BEANS_RELAY_S3_PREFIX` | A key prefix inside the bucket; empty by default. |

`Postgres` in the reference is the database service's name. `DATABASE_URL` reaches the database over the private network (`postgres.railway.internal`). Its certificate is self-signed and the relay checks certificates against the web's roots, so without `sslmode=disable` the relay exits at startup with `invalid peer certificate: UnknownIssuer`.

Attachments go to the bucket because a deploy replaces the container's disk. The relay addresses objects path-style (`<endpoint>/<bucket>/<key>`), which R2, S3, and MinIO accept; a Railway Bucket accepts it when its **Credentials** tab says path-style.

### Push notifications

| Variable | Value |
| --- | --- |
| `BEANS_RELAY_APNS_KEY` | The text of the APNs key from developer.apple.com › Keys (Apple Push Notifications service): paste the `.p8` file, `-----BEGIN PRIVATE KEY-----` line included. |
| `BEANS_RELAY_APNS_KEY_ID` | The key's 10-character id, also in the `.p8` file name. |
| `BEANS_RELAY_APNS_TEAM_ID` | The Apple team id. |
| `BEANS_RELAY_APNS_TOPIC` | The phone app's bundle id; `app.beans` by default. |
| `BEANS_RELAY_FCM_SERVICE_ACCOUNT` | The JSON of a key from Firebase console › Project settings › Service accounts. |

Both keys go in as text, so the service needs no volume and the Postgres setup keeps its deploys without a gap. A key with `\n` in place of its line breaks works too. Either variable also takes the path of a file (`/data/apns.p8`), for a relay that has a volume anyway. The relay's first log line shows `push=apns`, `push=fcm`, or `push=apns+fcm`.

### Optional

| Variable | Default | What it does |
| --- | --- | --- |
| `BEANS_RELAY_QUOTA_BYTES` | `5368709120` | Stored ciphertext allowed per identity, in bytes (5 GiB). `0` means no limit. |
| `BEANS_RELAY_CONCURRENT_UPLOADS` | `3` | Uploads over 1 MiB handled at once. Each takes some 80 MB while it is decoded and sent to the bucket; lower it on a small container. `0` means no limit. |
| `BEANS_RELAY_IP_PER_MINUTE` | `60` | Requests per minute one IP may make to registration, auth, and the pairing mailbox. |
| `BEANS_RELAY_IDENTITY_PER_SECOND` | `50` | Requests per second one identity may make across its machines, with a burst of ten times that. |
| `BEANS_RELAY_MIN_PROTOCOL` | `4` | Generic account-route floor; older clients get `426`. Roster PUTs always require at least 4, even when this is 3. Health advertises protocol 4, configured `min_protocol`, and effective `min_roster_protocol`. Upgrade every relay replica first, then paired cores, before relying on appearance sync. |
| `BEANS_RELAY_INACTIVE_DAYS` | `365` | An identity with no machine seen, no blob written, and no socket open for this many days is deleted with its attachments. Its Devices keep what they hold and register again if they come back. `0` keeps every identity. |
| `BEANS_RELAY_METRICS_TOKEN` | unset | Serves `GET /metrics` in Prometheus' text format to a scraper that sends this as a bearer token: `openssl rand -hex 32`. Unset, the route answers `404`. With more than one replica a scrape reaches one of them; request, push, and sweep counters carry its `instance`, and the totals are the same from each. |
| `RUST_LOG` | `info` | `info,beans_relay=debug` also logs each rate-limited request with the address it counted against. |

## Run the image elsewhere

The image builds from the repo root and listens on 8787 when `PORT` is unset. The variables above go in with `-e`.

```bash
docker build -f crates/relay/Dockerfile -t beans-relay .
```

```bash
docker run -p 8787:8787 -v beans-relay:/data beans-relay
```
