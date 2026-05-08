# osr-bot

Authorized low-frequency Twitch chat helper for a single configured channel.

## Purpose

The bot connects to one Twitch chat channel, listens for the configured trigger text, waits a short delay, and sends the configured response.

Default behavior:

- trigger: `game restarting`
- delay: `45` seconds
- response: `!play`
- cooldown after response: `60` seconds

The deployment is intentionally single-replica because multiple replicas could send duplicate chat messages.

## Runtime configuration

Required:

- `TWITCH_USERNAME`: Twitch username used by the app.
- `TWITCH_OAUTH_TOKEN`: Twitch OAuth token for the account. The app accepts both `oauth:<token>` and raw token formats.
- `TWITCH_CHANNEL`: target channel. The app accepts both `channel` and `#channel` formats.

Optional:

- `TRIGGER_TEXT`: defaults to `game restarting`.
- `RESPONSE_TEXT`: defaults to `!play`.
- `RESPONSE_DELAY_SECONDS`: defaults to `45`.
- `MIN_COOLDOWN_SECONDS`: defaults to `60`.
- `RUST_LOG`: defaults to `info`.

## Safety behavior

The app:

- only joins the configured channel;
- ignores messages from non-configured channels;
- lowercases chat messages before trigger matching;
- only reacts when the message contains the configured trigger text;
- keeps a `pending_play` flag in memory;
- ignores duplicate triggers while a response is pending;
- applies a short cooldown after sending the response.

Expected duplicate handling:

```text
12:00:00 -> game restarting
12:00:05 -> game restarting
12:00:10 -> game restarting
12:00:45 -> !play
```

Only one response should be sent.

## Image registry

The default image repository is GitHub Container Registry:

```text
ghcr.io/e-lemongrab/osr-bot
```

This avoids publishing the image under a personal Docker Hub namespace.

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

Do not increase replicas unless the app is changed to use shared distributed locking.

## Helm deploy

Example command shape:

```bash
helm upgrade --install osr-bot infra/chart/osr-bot \
  --namespace osr-bot \
  --create-namespace \
  --values infra/envs/values.yaml \
  --set-string image.repository="$IMAGE_REPOSITORY" \
  --set-string image.tag="$IMAGE_TAG" \
  --set-string config.twitchUsername="$TWITCH_USERNAME" \
  --set-string config.twitchOauthToken="$TWITCH_OAUTH_TOKEN" \
  --set-string config.twitchChannel="$TWITCH_CHANNEL" \
  --set-string config.triggerText="$TRIGGER_TEXT" \
  --set-string config.responseText="$RESPONSE_TEXT" \
  --set-string config.responseDelaySeconds="$RESPONSE_DELAY_SECONDS" \
  --set-string config.minCooldownSeconds="$MIN_COOLDOWN_SECONDS" \
  --wait \
  --timeout 120s
```

## CI/CD contract

Project variables expected by the deploy pipeline:

- `IMAGE_REPOSITORY`: defaults to `ghcr.io/e-lemongrab/osr-bot`.
- `TWITCH_USERNAME`
- `TWITCH_CHANNEL`
- `TRIGGER_TEXT`
- `RESPONSE_TEXT`
- `RESPONSE_DELAY_SECONDS`
- `MIN_COOLDOWN_SECONDS`

Project secrets expected by the deploy pipeline:

- `TWITCH_OAUTH_TOKEN`
- Kubernetes access credentials

For GHCR publishing from GitHub Actions, prefer the repository `GITHUB_TOKEN` with package write permissions instead of Docker Hub credentials.

The OAuth token must not be committed in clear text.

## Build

The container build context is the repository root and the Dockerfile is:

```text
src/osr-bot/docker/Dockerfile
```

Example image build:

```bash
docker build -f src/osr-bot/docker/Dockerfile -t ghcr.io/e-lemongrab/osr-bot:local .
```
