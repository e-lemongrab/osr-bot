mod oauth;

use anyhow::{bail, Context, Result};
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::{timeout, Instant as TokioInstant};
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::crypto::ring::default_provider;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

const TWITCH_IRC_HOST: &str = "irc.chat.twitch.tv";
const TWITCH_IRC_PORT: u16 = 6697;

#[derive(Debug, Clone)]
struct Config {
    twitch_username: String,
    twitch_channel: String,
    game_message_author: String,
    trigger_text: String,
    response_text: String,
    response_delay: Duration,
    min_cooldown: Duration,
    coordinate_command_text: Option<String>,
    post_play_coordinate_delay: Duration,
    death_trigger_text: String,
    death_coordinate_delay: Duration,
    removed_trigger_text: String,
    removed_play_delay: Duration,
    removed_coordinate_delay: Duration,
    min_coordinate_cooldown: Duration,
    irc_reconnect_interval: Duration,
    irc_reconnect_backoff: Duration,
    irc_reconnect_play_delay: Duration,
}

#[derive(Debug)]
struct BotState {
    pending_play: bool,
    last_play_sent: Option<Instant>,
    pending_coordinate: bool,
    last_coordinate_sent: Option<Instant>,
}

type SharedWriter = Arc<Mutex<WriteHalf<TlsStream<TcpStream>>>>;
type SharedState = Arc<Mutex<BotState>>;

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    install_rustls_crypto_provider();

    let config = Config::from_env()?;
    info!(
        twitch_username = %config.twitch_username,
        twitch_channel = %config.twitch_channel,
        game_message_author = %config.game_message_author,
        trigger_text = %config.trigger_text,
        response_text = %config.response_text,
        response_delay_seconds = config.response_delay.as_secs(),
        min_cooldown_seconds = config.min_cooldown.as_secs(),
        coordinate_enabled = config.coordinate_command_text.is_some(),
        post_play_coordinate_delay_seconds = config.post_play_coordinate_delay.as_secs(),
        death_trigger_text = %config.death_trigger_text,
        death_coordinate_delay_seconds = config.death_coordinate_delay.as_secs(),
        removed_trigger_text = %config.removed_trigger_text,
        removed_play_delay_seconds = config.removed_play_delay.as_secs(),
        removed_coordinate_delay_seconds = config.removed_coordinate_delay.as_secs(),
        min_coordinate_cooldown_seconds = config.min_coordinate_cooldown.as_secs(),
        irc_reconnect_interval_seconds = config.irc_reconnect_interval.as_secs(),
        irc_reconnect_backoff_seconds = config.irc_reconnect_backoff.as_secs(),
        irc_reconnect_play_delay_seconds = config.irc_reconnect_play_delay.as_secs(),
        "starting osr bot"
    );

    run(config).await
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

fn install_rustls_crypto_provider() {
    if default_provider().install_default().is_ok() {
        debug!("installed rustls ring crypto provider");
    }
}

impl Config {
    fn from_env() -> Result<Self> {
        let twitch_username = required_env("TWITCH_USERNAME")?.to_lowercase();
        let twitch_channel = normalize_channel(&required_env("TWITCH_CHANNEL")?);
        let game_message_author = optional_env("GAME_MESSAGE_AUTHOR")
            .map(|author| normalize_channel(&author))
            .unwrap_or_else(|| twitch_channel.clone());
        let trigger_text = env_or_default("TRIGGER_TEXT", "game is restarting").to_lowercase();
        let response_text = env_or_default("RESPONSE_TEXT", "!play");
        let response_delay = Duration::from_secs(env_or_u64("RESPONSE_DELAY_SECONDS", 45)?);
        let min_cooldown = Duration::from_secs(env_or_u64("MIN_COOLDOWN_SECONDS", 60)?);
        let coordinate_command_text = optional_env("COORDINATE_COMMAND_TEXT");
        let post_play_coordinate_delay =
            Duration::from_secs(env_or_u64("POST_PLAY_COORDINATE_DELAY_SECONDS", 5)?);
        let death_trigger_text = optional_env("DEATH_TRIGGER_TEXT")
            .unwrap_or_else(|| format!("@{twitch_username}, died"))
            .to_lowercase();
        let death_coordinate_delay =
            Duration::from_secs(env_or_u64("DEATH_COORDINATE_DELAY_SECONDS", 5)?);
        let removed_trigger_text = optional_env("REMOVED_TRIGGER_TEXT")
            .unwrap_or_else(|| format!("@{twitch_username}, you have been removed from the game"))
            .to_lowercase();
        let removed_play_delay = Duration::from_secs(env_or_u64("REMOVED_PLAY_DELAY_SECONDS", 30)?);
        let removed_coordinate_delay =
            Duration::from_secs(env_or_u64("REMOVED_COORDINATE_DELAY_SECONDS", 30)?);
        let min_coordinate_cooldown =
            Duration::from_secs(env_or_u64("MIN_COORDINATE_COOLDOWN_SECONDS", 10)?);
        let irc_reconnect_interval =
            Duration::from_secs(env_or_u64("IRC_RECONNECT_INTERVAL_SECONDS", 10_800)?);
        let irc_reconnect_backoff =
            Duration::from_secs(env_or_u64("IRC_RECONNECT_BACKOFF_SECONDS", 10)?);
        let irc_reconnect_play_delay =
            Duration::from_secs(env_or_u64("IRC_RECONNECT_PLAY_DELAY_SECONDS", 30)?);

        if trigger_text.trim().is_empty() {
            bail!("TRIGGER_TEXT cannot be empty");
        }

        if response_text.trim().is_empty() {
            bail!("RESPONSE_TEXT cannot be empty");
        }

        Ok(Self {
            twitch_username,
            twitch_channel,
            game_message_author,
            trigger_text,
            response_text,
            response_delay,
            min_cooldown,
            coordinate_command_text,
            post_play_coordinate_delay,
            death_trigger_text,
            death_coordinate_delay,
            removed_trigger_text,
            removed_play_delay,
            removed_coordinate_delay,
            min_coordinate_cooldown,
            irc_reconnect_interval,
            irc_reconnect_backoff,
            irc_reconnect_play_delay,
        })
    }
}

async fn resolve_chat_login_password() -> Result<String> {
    if let Some(refresh_value) = optional_env(concat!("TWITCH_", "REFRESH_TOKEN")) {
        let client_id = required_env("TWITCH_CLIENT_ID")?;
        let client_secret = required_env(concat!("TWITCH_CLIENT_", "SECRET"))?;
        let access_value =
            oauth::refresh_access_token(&client_id, &client_secret, &refresh_value).await?;
        return Ok(normalize_oauth_token(&access_value));
    }

    Ok(normalize_oauth_token(&required_env(concat!(
        "TWITCH_",
        "OAUTH_TOKEN"
    ))?))
}

async fn run(config: Config) -> Result<()> {
    let state = Arc::new(Mutex::new(BotState {
        pending_play: false,
        last_play_sent: None,
        pending_coordinate: false,
        last_coordinate_sent: None,
    }));

    let mut completed_sessions: u64 = 0;

    loop {
        let send_play_after_join = completed_sessions > 0;

        match run_irc_session(config.clone(), Arc::clone(&state), send_play_after_join).await {
            Ok(()) => warn!("Twitch IRC session ended; reconnecting"),
            Err(error) => error!(error = %error, "Twitch IRC session failed; reconnecting"),
        }

        completed_sessions = completed_sessions.saturating_add(1);
        tokio::time::sleep(config.irc_reconnect_backoff).await;
    }
}

async fn run_irc_session(
    config: Config,
    state: SharedState,
    send_play_after_join: bool,
) -> Result<()> {
    let chat_login_password = resolve_chat_login_password().await?;

    let tcp_stream = TcpStream::connect((TWITCH_IRC_HOST, TWITCH_IRC_PORT))
        .await
        .with_context(|| format!("failed to connect to {TWITCH_IRC_HOST}:{TWITCH_IRC_PORT}"))?;

    let tls_connector = build_tls_connector();
    let server_name = ServerName::try_from(TWITCH_IRC_HOST)
        .context("failed to build Twitch IRC TLS server name")?;
    let tls_stream = tls_connector
        .connect(server_name, tcp_stream)
        .await
        .context("failed to establish TLS connection to Twitch IRC")?;

    let (reader, writer) = tokio::io::split(tls_stream);
    let writer = Arc::new(Mutex::new(writer));

    authenticate_and_join(&writer, &config, &chat_login_password).await?;

    if send_play_after_join {
        schedule_play_response(
            config.clone(),
            Arc::clone(&writer),
            Arc::clone(&state),
            config.irc_reconnect_play_delay,
            config.post_play_coordinate_delay,
            "irc_reconnect",
        )
        .await?;
    }

    let reconnect_at = TokioInstant::now() + config.irc_reconnect_interval;
    let mut lines = BufReader::new(reader).lines();

    loop {
        let line = match timeout(
            reconnect_at.saturating_duration_since(TokioInstant::now()),
            lines.next_line(),
        )
        .await
        {
            Ok(result) => result.context("failed to read Twitch IRC line")?,
            Err(_) => {
                info!("planned Twitch IRC reconnect before token/session expiry");
                return Ok(());
            }
        };

        let Some(line) = line else {
            warn!("Twitch IRC stream ended");
            return Ok(());
        };

        debug!(irc_line = %line, "received IRC line");

        if let Some(payload) = line.strip_prefix("PING ") {
            let pong = format!("PONG {payload}");
            write_irc_line(&writer, &pong).await?;
            debug!("responded to Twitch IRC PING");
            continue;
        }

        if let Some(message) = ChatMessage::parse(&line) {
            handle_chat_message(message, &config, Arc::clone(&writer), Arc::clone(&state)).await?;
        }
    }
}

fn build_tls_connector() -> TlsConnector {
    let root_store = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    TlsConnector::from(Arc::new(config))
}

async fn authenticate_and_join(
    writer: &SharedWriter,
    config: &Config,
    chat_login_password: &str,
) -> Result<()> {
    write_irc_line(writer, &format!("PASS {chat_login_password}")).await?;
    write_irc_line(writer, &format!("NICK {}", config.twitch_username)).await?;
    write_irc_line(writer, &format!("JOIN #{}", config.twitch_channel)).await?;

    info!(twitch_channel = %config.twitch_channel, "joined Twitch channel");
    Ok(())
}

async fn handle_chat_message(
    message: ChatMessage,
    config: &Config,
    writer: SharedWriter,
    state: SharedState,
) -> Result<()> {
    if message.channel != config.twitch_channel {
        debug!(
            message_channel = %message.channel,
            configured_channel = %config.twitch_channel,
            "ignored message from non-configured channel"
        );
        return Ok(());
    }

    let normalized_body = message.body.to_lowercase();

    if is_untrusted_game_trigger(&message, &normalized_body, config) {
        warn!(
            sender = %message.sender,
            expected_author = %config.game_message_author,
            message_channel = %message.channel,
            "ignored spoofable game trigger from untrusted Twitch user"
        );
        return Ok(());
    }

    if normalized_body.contains(&config.removed_trigger_text) {
        schedule_play_response(
            config.clone(),
            Arc::clone(&writer),
            Arc::clone(&state),
            config.removed_play_delay,
            config.removed_coordinate_delay,
            "removed",
        )
        .await?;
        return Ok(());
    }

    if normalized_body.contains(&config.death_trigger_text) {
        schedule_coordinate_command(
            config.clone(),
            Arc::clone(&writer),
            Arc::clone(&state),
            config.death_coordinate_delay,
            "death",
        )
        .await?;
    }

    if !normalized_body.contains(&config.trigger_text) {
        return Ok(());
    }

    schedule_play_response(
        config.clone(),
        writer,
        state,
        config.response_delay,
        config.post_play_coordinate_delay,
        "restart",
    )
    .await
}

fn contains_game_trigger(normalized_body: &str, config: &Config) -> bool {
    normalized_body.contains(&config.removed_trigger_text)
        || normalized_body.contains(&config.death_trigger_text)
        || normalized_body.contains(&config.trigger_text)
}

fn is_untrusted_game_trigger(
    message: &ChatMessage,
    normalized_body: &str,
    config: &Config,
) -> bool {
    contains_game_trigger(normalized_body, config) && message.sender != config.game_message_author
}

async fn schedule_play_response(
    config: Config,
    writer: SharedWriter,
    state: SharedState,
    play_delay: Duration,
    coordinate_delay: Duration,
    reason: &'static str,
) -> Result<()> {
    let now = Instant::now();
    {
        let mut state_guard = state.lock().await;

        if state_guard.pending_play {
            info!(
                reason,
                "ignored play response because one is already pending"
            );
            return Ok(());
        }

        if let Some(last_play_sent) = state_guard.last_play_sent {
            let elapsed = now.saturating_duration_since(last_play_sent);
            if elapsed < config.min_cooldown {
                info!(
                    reason,
                    elapsed_seconds = elapsed.as_secs(),
                    min_cooldown_seconds = config.min_cooldown.as_secs(),
                    "ignored play response because cooldown is active"
                );
                return Ok(());
            }
        }

        state_guard.pending_play = true;
    }

    info!(
        reason,
        delay_seconds = play_delay.as_secs(),
        response_text = %config.response_text,
        "scheduling play response"
    );

    tokio::spawn(async move {
        tokio::time::sleep(play_delay).await;

        let result = async {
            write_irc_line(
                &writer,
                &format!(
                    "PRIVMSG #{} :{}",
                    config.twitch_channel, config.response_text
                ),
            )
            .await?;

            let mut state_guard = state.lock().await;
            state_guard.last_play_sent = Some(Instant::now());
            state_guard.pending_play = false;
            drop(state_guard);

            info!(reason, response_text = %config.response_text, "sent Twitch chat response");

            schedule_coordinate_command(
                config.clone(),
                Arc::clone(&writer),
                Arc::clone(&state),
                coordinate_delay,
                reason,
            )
            .await?;

            Ok::<(), anyhow::Error>(())
        }
        .await;

        if let Err(err) = result {
            error!(reason, error = %err, "failed to send Twitch chat response or follow-up coordinate command");
            let mut state_guard = state.lock().await;
            state_guard.pending_play = false;
        }
    });

    Ok(())
}

async fn schedule_coordinate_command(
    config: Config,
    writer: SharedWriter,
    state: SharedState,
    delay: Duration,
    reason: &'static str,
) -> Result<()> {
    let Some(coordinate_command_text) = config.coordinate_command_text.clone() else {
        debug!(reason, "coordinate command disabled");
        return Ok(());
    };

    let now = Instant::now();
    {
        let mut state_guard = state.lock().await;

        if state_guard.pending_coordinate {
            info!(
                reason,
                "ignored coordinate command because one is already pending"
            );
            return Ok(());
        }

        if let Some(last_coordinate_sent) = state_guard.last_coordinate_sent {
            let elapsed = now.saturating_duration_since(last_coordinate_sent);
            if elapsed < config.min_coordinate_cooldown {
                info!(
                    reason,
                    elapsed_seconds = elapsed.as_secs(),
                    min_coordinate_cooldown_seconds = config.min_coordinate_cooldown.as_secs(),
                    "ignored coordinate command because cooldown is active"
                );
                return Ok(());
            }
        }

        state_guard.pending_coordinate = true;
    }

    info!(
        reason,
        delay_seconds = delay.as_secs(),
        "scheduling coordinate command"
    );

    tokio::spawn(async move {
        tokio::time::sleep(delay).await;

        let result = async {
            write_irc_line(
                &writer,
                &format!(
                    "PRIVMSG #{} :{}",
                    config.twitch_channel, coordinate_command_text
                ),
            )
            .await?;

            let mut state_guard = state.lock().await;
            state_guard.last_coordinate_sent = Some(Instant::now());
            state_guard.pending_coordinate = false;

            info!(reason, "sent coordinate command");
            Ok::<(), anyhow::Error>(())
        }
        .await;

        if let Err(err) = result {
            error!(reason, error = %err, "failed to send coordinate command");
            let mut state_guard = state.lock().await;
            state_guard.pending_coordinate = false;
        }
    });

    Ok(())
}

async fn write_irc_line(writer: &SharedWriter, line: &str) -> Result<()> {
    let mut writer = writer.lock().await;
    writer
        .write_all(format!("{line}\r\n").as_bytes())
        .await
        .with_context(|| format!("failed to write IRC line: {line}"))?;
    writer.flush().await.context("failed to flush IRC writer")?;
    Ok(())
}

#[derive(Debug)]
struct ChatMessage {
    sender: String,
    channel: String,
    body: String,
}

impl ChatMessage {
    fn parse(line: &str) -> Option<Self> {
        let line = if line.starts_with('@') {
            line.split_once(' ')?.1
        } else {
            line
        };

        let after_prefix = line.strip_prefix(':')?;
        let (prefix, after_prefix) = after_prefix.split_once(' ')?;
        let sender = prefix.split_once('!')?.0.to_lowercase();

        let after_privmsg = after_prefix.strip_prefix("PRIVMSG #")?;
        let separator = after_privmsg.find(" :")?;

        let channel = after_privmsg[..separator].to_lowercase();
        let body = after_privmsg[separator + 2..].to_string();

        Some(Self {
            sender,
            channel,
            body,
        })
    }
}

fn required_env(key: &str) -> Result<String> {
    let value = env::var(key).with_context(|| format!("missing required env var {key}"))?;
    let value = value.trim().to_string();

    if value.is_empty() {
        bail!("env var {key} cannot be empty");
    }

    Ok(value)
}

fn optional_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_or_default(key: &str, default: &str) -> String {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_or_u64(key: &str, default: u64) -> Result<u64> {
    match env::var(key) {
        Ok(value) if !value.trim().is_empty() => value
            .trim()
            .parse::<u64>()
            .with_context(|| format!("env var {key} must be an unsigned integer")),
        _ => Ok(default),
    }
}

fn normalize_channel(channel: &str) -> String {
    channel.trim().trim_start_matches('#').to_lowercase()
}

fn normalize_oauth_token(token: &str) -> String {
    let token = token.trim();
    if token.starts_with("oauth:") {
        token.to_string()
    } else {
        format!("oauth:{token}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            twitch_username: "burzvarg".to_string(),
            twitch_channel: "onestreamrpg".to_string(),
            game_message_author: "onestreamrpg".to_string(),
            trigger_text: "game is restarting".to_string(),
            response_text: "!play".to_string(),
            response_delay: Duration::from_secs(45),
            min_cooldown: Duration::from_secs(60),
            coordinate_command_text: Some("!coord".to_string()),
            post_play_coordinate_delay: Duration::from_secs(5),
            death_trigger_text: "@burzvarg, died".to_string(),
            death_coordinate_delay: Duration::from_secs(5),
            removed_trigger_text: "@burzvarg, you have been removed from the game".to_string(),
            removed_play_delay: Duration::from_secs(30),
            removed_coordinate_delay: Duration::from_secs(30),
            min_coordinate_cooldown: Duration::from_secs(10),
            irc_reconnect_interval: Duration::from_secs(10_800),
            irc_reconnect_backoff: Duration::from_secs(10),
            irc_reconnect_play_delay: Duration::from_secs(30),
        }
    }

    #[test]
    fn parses_privmsg_without_tags() {
        let message = ChatMessage::parse(
            ":crapfairy!crapfairy@crapfairy.tmi.twitch.tv PRIVMSG #onestreamrpg :hello chat",
        )
        .expect("PRIVMSG should parse");

        assert_eq!(message.sender, "crapfairy");
        assert_eq!(message.channel, "onestreamrpg");
        assert_eq!(message.body, "hello chat");
    }

    #[test]
    fn parses_privmsg_with_ircv3_tags() {
        let message = ChatMessage::parse(
            "@badge-info=;badges=;color=#1E90FF :crapfairy!crapfairy@crapfairy.tmi.twitch.tv PRIVMSG #onestreamrpg :hello with tags",
        )
        .expect("tagged PRIVMSG should parse");

        assert_eq!(message.sender, "crapfairy");
        assert_eq!(message.channel, "onestreamrpg");
        assert_eq!(message.body, "hello with tags");
    }

    #[test]
    fn flags_spoofed_game_trigger_from_untrusted_sender() {
        let config = test_config();
        let message = ChatMessage {
            sender: "crapfairy".to_string(),
            channel: "onestreamrpg".to_string(),
            body: "onestreamrpg: @burzvarg, You have been removed from the game".to_string(),
        };
        let normalized_body = message.body.to_lowercase();

        assert!(is_untrusted_game_trigger(
            &message,
            &normalized_body,
            &config
        ));
    }

    #[test]
    fn accepts_game_trigger_from_configured_sender() {
        let config = test_config();
        let message = ChatMessage {
            sender: "onestreamrpg".to_string(),
            channel: "onestreamrpg".to_string(),
            body: "onestreamrpg: @burzvarg, You have been removed from the game".to_string(),
        };
        let normalized_body = message.body.to_lowercase();

        assert!(contains_game_trigger(&normalized_body, &config));
        assert!(!is_untrusted_game_trigger(
            &message,
            &normalized_body,
            &config
        ));
    }
}
