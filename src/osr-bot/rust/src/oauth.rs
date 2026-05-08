use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tracing::{info, warn};

const TWITCH_TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";

#[derive(Debug, Deserialize)]
struct TwitchRefreshResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    scope: Option<Vec<String>>,
    token_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TwitchErrorResponse {
    error: Option<String>,
    status: Option<u16>,
    message: Option<String>,
}

pub async fn refresh_access_token(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<String> {
    let response = reqwest::Client::new()
        .post(TWITCH_TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ])
        .send()
        .await
        .context("failed to call Twitch token refresh endpoint")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read Twitch token refresh response")?;

    if !status.is_success() {
        let message = serde_json::from_str::<TwitchErrorResponse>(&body)
            .ok()
            .and_then(|error| error.message.or(error.error).or_else(|| error.status.map(|s| s.to_string())))
            .unwrap_or_else(|| format!("HTTP {status}"));
        bail!("Twitch token refresh failed: {message}");
    }

    let refreshed = serde_json::from_str::<TwitchRefreshResponse>(&body)
        .context("failed to parse Twitch token refresh response")?;

    if let Some(expires_in) = refreshed.expires_in {
        info!(expires_in_seconds = expires_in, "refreshed Twitch access token");
    } else {
        info!("refreshed Twitch access token");
    }

    if let Some(token_type) = refreshed.token_type.as_deref() {
        if token_type != "bearer" {
            warn!(token_type = %token_type, "unexpected Twitch token type");
        }
    }

    if let Some(scope) = refreshed.scope.as_ref() {
        info!(scopes = ?scope, "received Twitch token scopes");
    }

    if let Some(new_refresh_token) = refreshed.refresh_token.as_deref() {
        if new_refresh_token != refresh_token {
            warn!("Twitch returned a rotated refresh token; update TWITCH_REFRESH_TOKEN in project secrets");
        }
    }

    Ok(refreshed.access_token)
}
