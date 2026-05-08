mod oauth;

use anyhow::{bail, Context, Result};
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
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
    twitch_oauth_token: String,
    twitch_channel: String,
    trigger_text: String,
    response_text: String,
    response_delay: Duration,
    min_cooldown: Duration,
}

#[derive(Debug)]
struct BotState {
    pending_play: bool,
    last_play_sent: Option<Instant>,
}

type SharedWriter = Arc<Mutex<WriteHalf<TlsStream<TcpStream>>>>;
type SharedState = Arc<Mutex<BotState>>;

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    install_rustls_crypto_provider();

    let config = Config::from_env().await?;
    info!(
        twitch_username = %config.twitch_username,
        twitch_channel = %config.twitch_channel,
        trigger_text = %config.trigger_text,
        response_text = %config.response_text,
        response_delay_seconds = config.response_delay.as_secs(),
        min_cooldown_seconds = config.min_cooldown.as_secs(),
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
    async fn from_env() -> Result<Self> {
        let twitch_username = required_env("TWITCH_USERNAME")?.to_lowercase();
        let twitch_oauth_token = resolve_twitch_oauth_token().await?;
        let twitch_channel = normalize_channel(&required_env("TWITCH_CHANNEL")?);
        let trigger_text = env_or_default("TRIGGER_TEXT", "game restarting").to_lowercase();
        let response_text = env_or_default("RESPONSE_TEXT", "!play");
        let response_delay = Duration::from_secs(env_or_u64("RESPONSE_DELAY_SECONDS", 45)?);
        let min_cooldown = Duration::from_secs(env_or_u64("MIN_COOLDOWN_SECONDS", 60)?);

        if trigger_text.trim().is_empty() {
            bail!("TRIGGER_TEXT cannot be empty");
        }

        if response_text.trim().is_empty() {
            bail!("RESPONSE_TEXT cannot be empty");
        }

        Ok(Self {
            twitch_username,
            twitch_oauth_token,
            twitch_channel,
            trigger_text,
            response_text,
            response_delay,
            min_cooldown,
        })
    }
}

async fn resolve_twitch_oauth_token() -> Result<String> {
    if let Some(refresh_token) = optional_env("TWITCH_REFRESH_TOKEN") {
        let client_id = required_env("TWITCH_CLIENT_ID")?;
        let client_secret = required_env("TWITCH_CLIENT_SECRET")?;
        let access_token = oauth::refresh_access_token(&client_id, &client_secret, &refresh_token).await?;
        return Ok(normalize_oauth_token(&access_token));
    }

    Ok(normalize_oauth_token(&required_env("TWITCH_OAUTH_TOKEN")?))
}

async fn run(config: Config) -> Result<()> {
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
    let state = Arc::new(Mutex::new(BotState {
        pending_play: false,
        last_play_sent: None,
    }));

    authenticate_and_join(&writer, &config).await?;

    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .context("failed to read Twitch IRC line")?
    {
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

    warn!("Twitch IRC stream ended");
    Ok(())
}

fn build_tls_connector() -> TlsConnector {
    let root_store = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    TlsConnector::from(Arc::new(config))
}

async fn authenticate_and_join(writer: &SharedWriter, config: &Config) -> Result<()> {
    write_irc_line(writer, &format!("PASS {}", config.twitch_oauth_token)).await?;
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
    if !normalized_body.contains(&config.trigger_text) {
        return Ok(());
    }

    let now = Instant::now();
    {
        let mut state_guard = state.lock().await;

        if state_guard.pending_play {
            info!("ignored trigger because response is already pending");
            return Ok(());
        }

        if let Some(last_play_sent) = state_guard.last_play_sent {
            let elapsed = now.saturating_duration_since(last_play_sent);
            if elapsed < config.min_cooldown {
                info!(
                    elapsed_seconds = elapsed.as_secs(),
                    min_cooldown_seconds = config.min_cooldown.as_secs(),
                    "ignored trigger because cooldown is active"
                );
                return Ok(());
            }
        }

        state_guard.pending_play = true;
    }

    info!(
        delay_seconds = config.response_delay.as_secs(),
        response_text = %config.response_text,
        "trigger detected; scheduling response"
    );

    let config = config.clone();
    tokio::spawn(async move {
        tokio::time::sleep(config.response_delay).await;

        let result = async {
            write_irc_line(
                &writer,
                &format!("PRIVMSG #{} :{}", config.twitch_channel, config.response_text),
            )
            .await?;

            let mut state_guard = state.lock().await;
            state_guard.last_play_sent = Some(Instant::now());
            state_guard.pending_play = false;

            info!(response_text = %config.response_text, "sent Twitch chat response");
            Ok::<(), anyhow::Error>(())
        }
        .await;

        if let Err(err) = result {
            error!(error = %err, "failed to send Twitch chat response");
            let mut state_guard = state.lock().await;
            state_guard.pending_play = false;
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
    channel: String,
    body: String,
}

impl ChatMessage {
    fn parse(line: &str) -> Option<Self> {
        let privmsg_index = line.find(" PRIVMSG #")?;
        let after_privmsg = &line[privmsg_index + " PRIVMSG #".len()..];
        let separator = after_privmsg.find(" :")?;

        let channel = after_privmsg[..separator].to_lowercase();
        let body = after_privmsg[separator + 2..].to_string();

        Some(Self { channel, body })
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
