# osr-bot

Authorized low-frequency Twitch chat helper for a single configured channel.

## Purpose

`osr-bot` connects to one Twitch chat channel, listens for a configured restart message, waits a short delay, and sends a configured response.

Current default behavior:

- trigger: `game is restarting`
- delay: `45` seconds
- response: `!play`
- cooldown after response: `60` seconds

Beyond the restart trigger, the bot also reacts to two other game events and to its own reconnects:

- **death**: on `DEATH_TRIGGER_TEXT` (default `@<username>, died`) it sends only the coordinate command, after `DEATH_COORDINATE_DELAY_SECONDS`.
- **removed from game**: on `REMOVED_TRIGGER_TEXT` (default `@<username>, you have been removed from the game`) it re-sends the play response after `REMOVED_PLAY_DELAY_SECONDS`, then the coordinate command.
- **reconnect**: the IRC session is deliberately rebuilt every `IRC_RECONNECT_INTERVAL_SECONDS` so the access token is refreshed before it can expire mid-session. On every session after the first, the play response is re-sent after `IRC_RECONNECT_PLAY_DELAY_SECONDS`.

The coordinate command is optional. Leaving `COORDINATE_COMMAND_TEXT` empty disables that whole path.

The app is intentionally designed as a single long-running process with in-memory state. It does not use a database.

## Safety model

The bot is not intended for spam, bot viewers, mass automation, or multi-channel use.

The app:

- only joins the configured Twitch channel;
- only processes messages from that configured channel;
- lowercases chat messages before trigger matching;
- only reacts when the message contains a configured trigger text;
- rejects any message carrying a game trigger whose sender is not `GAME_MESSAGE_AUTHOR`, so a viewer cannot spoof a game event;
- keeps separate pending flags in memory for the play response and the coordinate command;
- ignores duplicate triggers while the corresponding response is pending;
- applies a short cooldown after sending each of them;
- runs as a single Kubernetes replica.

Expected duplicate handling:

```text
12:00:00 -> game is restarting
12:00:05 -> game is restarting
12:00:10 -> game is restarting
12:00:45 -> !play
```

Only one response should be sent.

## Runtime configuration

All deployment-time configuration is expected to come from GitHub Actions secrets and Helm `--set-string` values. Do not commit real tokens, kubeconfigs, PATs, or channel-specific sensitive data to the repository.

Required Twitch configuration:

- `TWITCH_USERNAME`: Twitch username used by the app.
- `TWITCH_CHANNEL`: target channel. The app accepts both `channel` and `#channel` formats.

Preferred OAuth mode:

- `TWITCH_CLIENT_ID`: Twitch app Client ID.
- `TWITCH_CLIENT_SECRET`: Twitch app Client Secret.
- `TWITCH_REFRESH_TOKEN`: Twitch user refresh token for the account that writes to chat.

Fallback OAuth mode:

- `TWITCH_OAUTH_TOKEN`: Twitch access token for the account. The app accepts both `oauth:<token>` and raw token formats.

If `TWITCH_REFRESH_TOKEN` is present, the app refreshes a new access token at startup and ignores `TWITCH_OAUTH_TOKEN` for authentication. If Twitch returns a rotated refresh token, the app logs a warning without printing the token value.

Behavior configuration:

- `TRIGGER_TEXT`: defaults to `game is restarting`.
- `GAME_MESSAGE_AUTHOR`: trusted Twitch login for game event messages. Defaults to `TWITCH_CHANNEL`.
- `RESPONSE_TEXT`: defaults to `!play`.
- `RESPONSE_DELAY_SECONDS`: defaults to `45`.
- `MIN_COOLDOWN_SECONDS`: defaults to `60`.
- `COORDINATE_COMMAND_TEXT`: no default. Empty disables the coordinate command entirely.
- `POST_PLAY_COORDINATE_DELAY_SECONDS`: defaults to `5`.
- `MIN_COORDINATE_COOLDOWN_SECONDS`: defaults to `10`.
- `DEATH_TRIGGER_TEXT`: defaults to `@<TWITCH_USERNAME>, died`.
- `DEATH_COORDINATE_DELAY_SECONDS`: defaults to `5`.
- `REMOVED_TRIGGER_TEXT`: defaults to `@<TWITCH_USERNAME>, you have been removed from the game`.
- `REMOVED_PLAY_DELAY_SECONDS`: defaults to `30`.
- `REMOVED_COORDINATE_DELAY_SECONDS`: defaults to `30`.
- `RUST_LOG`: defaults to `info`.

Read by the app but **not exposed by the Helm chart**, so in a deployed pod they are always their compiled defaults:

- `IRC_RECONNECT_INTERVAL_SECONDS`: defaults to `10800` (3 hours).
- `IRC_RECONNECT_BACKOFF_SECONDS`: defaults to `10`.
- `IRC_RECONNECT_PLAY_DELAY_SECONDS`: defaults to `30`.

## Kubernetes

The Helm chart lives in:

```text
infra/chart/osr-bot
```

The single environment values file lives in:

```text
infra/envs/values.yaml
```

The Deployment hardcodes:

```yaml
replicas: 1
```

Do not increase replicas unless the app is changed to use shared distributed locking. Multiple replicas could send duplicate chat messages.

The pod runs as an unprivileged user and logs to stdout.

## Image registry

The default image repository is GitHub Container Registry:

```text
ghcr.io/e-lemongrab/osr-bot
```

The package can remain private. For private GHCR pulls, the deploy workflow creates or updates a Kubernetes `docker-registry` pull secret from GitHub Actions secrets and passes it to Helm as `imagePullSecrets[0].name`.

The container image must not contain secrets. Runtime secrets are injected only at deploy time.

## CI/CD

Deployment is handled by:

```text
.github/workflows/deploy.yaml
```

The workflow is manual via `workflow_dispatch` and accepts an `image_tag` input.

Pipeline flow:

1. Cross-compile the Rust binary for ARM64 using `aarch64-unknown-linux-gnu`.
2. Stage the binary under `dist/osr-bot/osr-bot`.
3. Build a runtime-only Docker image.
4. Push the image to GHCR.
5. Write kubeconfig from `KUBE_CONFIG_B64`.
6. Create/update the GHCR image pull secret in Kubernetes.
7. Deploy with Helm.

The Dockerfile is runtime-only. It does not run `cargo build` inside Docker.

## Required GitHub Actions secrets

General deploy secrets:

- `IMAGE_REPOSITORY`: for example `ghcr.io/e-lemongrab/osr-bot`.
- `NAMESPACE`: for example `osr-bot`.
- `KUBE_CONFIG_B64`: base64-encoded kubeconfig for the deploy user.

GHCR pull secrets:

- `GHCR_USERNAME`
- `GHCR_PAT`: PAT with package read access.
- `GHCR_EMAIL`
- `IMAGE_PULL_SECRET_NAME`: for example `ghcr-pull-secret`.

Twitch secrets:

- `TWITCH_USERNAME`
- `TWITCH_CHANNEL`
- `TWITCH_CLIENT_ID`
- `TWITCH_CLIENT_SECRET`
- `TWITCH_REFRESH_TOKEN`
- `TWITCH_OAUTH_TOKEN`: fallback access token.
- `TRIGGER_TEXT`
- `RESPONSE_TEXT`
- `RESPONSE_DELAY_SECONDS`
- `MIN_COOLDOWN_SECONDS`
- `COORDINATE_COMMAND_TEXT`
- `POST_PLAY_COORDINATE_DELAY_SECONDS`
- `DEATH_TRIGGER_TEXT`
- `DEATH_COORDINATE_DELAY_SECONDS`
- `MIN_COORDINATE_COOLDOWN_SECONDS`

Known gap: `GAME_MESSAGE_AUTHOR` is the trust anchor for the anti-spoofing check, but the deploy workflow never passes it and `infra/envs/values.yaml` has no `gameMessageAuthor` key, so the deployed pod receives an empty value and the app falls back to `TWITCH_CHANNEL`. That fallback is correct only while the game posts under the channel's own login.

The `removed*` values are likewise not passed by the workflow; they come from `infra/envs/values.yaml`.

## Public repository safety checklist

Before making this repository public, verify:

- no real OAuth tokens are committed;
- no refresh tokens are committed;
- no GitHub PATs are committed;
- no kubeconfig is committed;
- no `.env` files are committed;
- no generated Kubernetes Secret manifests with real data are committed;
- no workflow logs contain manually printed secrets;
- no screenshots or markdown files contain private operational data.

The repository should be safe to make public if all sensitive values remain only in GitHub Actions secrets and Kubernetes secrets.

## Local build notes

The container build expects the ARM64 binary to exist at:

```text
dist/osr-bot/osr-bot
```

The CI workflow creates this file before Docker packaging.

Example local ARM64 build shape:

```bash
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu --manifest-path src/osr-bot/rust/Cargo.toml
mkdir -p dist/osr-bot
cp target/aarch64-unknown-linux-gnu/release/osr-bot dist/osr-bot/osr-bot
docker build -f src/osr-bot/docker/Dockerfile -t ghcr.io/e-lemongrab/osr-bot:local .
```
