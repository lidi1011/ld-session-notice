use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration as StdDuration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout, Duration, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::app_state::AppState;
use crate::domain::MochiVoiceConfig;
use crate::storage;

const ENABLED_KEY: &str = "mochi_voice_enabled";
const SERIAL_PORT_KEY: &str = "mochi_voice_serial_port";
const ASR_URL_KEY: &str = "mochi_voice_asr_url";
const LAST_STATUS_KEY: &str = "mochi_voice_last_status";

const DEFAULT_SERIAL_PORT: &str = "/dev/cu.usbmodem1301";
const DEFAULT_ASR_URL: &str = "ws://110.42.235.130:10095";
const SERIAL_BAUD: u32 = 921_600;
const MAGIC: &[u8; 4] = b"MASR";
const MAX_PAYLOAD_LEN: usize = 16 * 1024;
const MAX_RECORDING_DURATION: Duration = Duration::from_secs(12);

#[derive(Debug)]
enum MochiPacket {
    Start,
    Audio(Vec<u8>),
    End,
}

type AsrSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct AsrSession {
    socket: AsrSocket,
    latest_text: String,
}

pub async fn config(pool: &SqlitePool) -> anyhow::Result<MochiVoiceConfig> {
    Ok(MochiVoiceConfig {
        enabled: storage::bool_setting(pool, ENABLED_KEY, false).await?,
        serial_port: storage::get_setting(pool, SERIAL_PORT_KEY)
            .await?
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SERIAL_PORT.to_string()),
        asr_url: storage::get_setting(pool, ASR_URL_KEY)
            .await?
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ASR_URL.to_string()),
        last_status: storage::get_setting(pool, LAST_STATUS_KEY).await?,
    })
}

pub async fn save_config(
    pool: &SqlitePool,
    enabled: bool,
    serial_port: Option<String>,
    asr_url: Option<String>,
) -> anyhow::Result<MochiVoiceConfig> {
    storage::put_setting(pool, ENABLED_KEY, if enabled { "true" } else { "false" }).await?;
    storage::put_setting(
        pool,
        SERIAL_PORT_KEY,
        normalized_or_default(serial_port.as_deref(), DEFAULT_SERIAL_PORT).as_str(),
    )
    .await?;
    storage::put_setting(
        pool,
        ASR_URL_KEY,
        normalized_or_default(asr_url.as_deref(), DEFAULT_ASR_URL).as_str(),
    )
    .await?;

    if enabled {
        set_status(
            pool,
            "Mochi voice input enabled; close Arduino Serial Monitor if the serial port is busy",
        )
        .await?;
    } else {
        set_status(pool, "Mochi voice input disabled").await?;
    }

    config(pool).await
}

pub fn start(state: AppState) {
    tauri::async_runtime::spawn(async move {
        run(state).await;
    });
}

async fn run(state: AppState) {
    loop {
        match config(&state.pool).await {
            Ok(config) if config.enabled => {
                if let Err(error) = run_serial_session(state.pool.clone(), config).await {
                    let message = format!("Mochi voice input stopped: {error}");
                    eprintln!("{message}");
                    let _ = set_status(&state.pool, &message).await;
                    sleep(Duration::from_secs(2)).await;
                }
            }
            Ok(_) => sleep(Duration::from_secs(2)).await,
            Err(error) => {
                eprintln!("Mochi voice config failed: {error}");
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn run_serial_session(
    pool: SqlitePool,
    session_config: MochiVoiceConfig,
) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::channel::<Result<MochiPacket, String>>(32);
    let stop = Arc::new(AtomicBool::new(false));
    let reader_stop = stop.clone();
    let port = session_config.serial_port.clone();

    set_status(
        &pool,
        &format!(
            "Listening for Mochi voice input on {} @ {}",
            session_config.serial_port, SERIAL_BAUD
        ),
    )
    .await?;

    let reader = tokio::task::spawn_blocking(move || serial_reader(port, tx, reader_stop));
    let mut asr: Option<AsrSession> = None;
    let mut recording_started_at: Option<Instant> = None;

    loop {
        tokio::select! {
            Some(item) = rx.recv() => {
                match item {
                    Ok(MochiPacket::Start) => {
                        set_status(&pool, "Mochi recording started").await?;
                        match AsrSession::connect(&session_config.asr_url).await {
                            Ok(session) => {
                                asr = Some(session);
                                recording_started_at = Some(Instant::now());
                            }
                            Err(error) => {
                                let message = format!("ASR connection failed: {error}");
                                set_status(&pool, &message).await?;
                                asr = None;
                                recording_started_at = None;
                            }
                        }
                    }
                    Ok(MochiPacket::Audio(bytes)) => {
                        if let Some(session) = asr.as_mut() {
                            if let Err(error) = session.send_audio(bytes).await {
                                let message = format!("ASR audio send failed: {error}");
                                set_status(&pool, &message).await?;
                                asr = None;
                                recording_started_at = None;
                            } else if recording_started_at
                                .map(|started_at| started_at.elapsed() >= MAX_RECORDING_DURATION)
                                .unwrap_or(false)
                            {
                                set_status(&pool, "Mochi recording timed out; transcribing").await?;
                                if let Some(mut session) = asr.take() {
                                    finish_asr_session(&pool, &mut session).await?;
                                }
                                recording_started_at = None;
                            }
                        }
                    }
                    Ok(MochiPacket::End) => {
                        set_status(&pool, "Mochi recording ended; transcribing").await?;
                        if let Some(mut session) = asr.take() {
                            finish_asr_session(&pool, &mut session).await?;
                        }
                        recording_started_at = None;
                    }
                    Err(error) => anyhow::bail!(error),
                }
            }
            _ = sleep(Duration::from_secs(1)) => {
                let latest = config(&pool).await?;
                if !same_session_config(&session_config, &latest) {
                    stop.store(true, Ordering::Relaxed);
                    set_status(&pool, "Restarting Mochi voice input with updated settings").await?;
                    break;
                }
            }
            else => break,
        }
    }

    stop.store(true, Ordering::Relaxed);
    let _ = reader.await;
    Ok(())
}

async fn finish_asr_session(pool: &SqlitePool, session: &mut AsrSession) -> anyhow::Result<()> {
    match session.finish().await {
        Ok(text) if !text.trim().is_empty() => {
            let text = text.trim().to_string();
            paste_text(text.clone()).await?;
            set_status(pool, &format!("Inserted transcript: {text}")).await?;
        }
        Ok(_) => set_status(pool, "ASR returned no transcript").await?,
        Err(error) => {
            let message = format!("ASR transcription failed: {error}");
            set_status(pool, &message).await?;
        }
    }
    Ok(())
}

impl AsrSession {
    async fn connect(asr_url: &str) -> anyhow::Result<Self> {
        let (mut socket, _) = connect_async(asr_url).await?;
        let init = json!({
            "mode": "online",
            "chunk_size": [5, 10, 5],
            "chunk_interval": 10,
            "wav_name": "mochi",
            "is_speaking": true
        });
        socket.send(Message::Text(init.to_string().into())).await?;
        Ok(Self {
            socket,
            latest_text: String::new(),
        })
    }

    async fn send_audio(&mut self, bytes: Vec<u8>) -> anyhow::Result<()> {
        self.socket.send(Message::Binary(bytes.into())).await?;
        self.collect_for(Duration::from_millis(5)).await?;
        Ok(())
    }

    async fn finish(&mut self) -> anyhow::Result<String> {
        self.socket
            .send(Message::Text(
                json!({ "is_speaking": false }).to_string().into(),
            ))
            .await?;
        self.collect_for(Duration::from_secs(6)).await?;
        let _ = self.socket.close(None).await;
        Ok(self.latest_text.clone())
    }

    async fn collect_for(&mut self, duration: Duration) -> anyhow::Result<()> {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait = remaining.min(Duration::from_millis(300));
            match timeout(wait, self.socket.next()).await {
                Ok(Some(Ok(message))) => {
                    if let Some((text, is_final)) = transcript_from_message(&message) {
                        if !text.trim().is_empty() {
                            self.latest_text = text;
                        }
                        if is_final {
                            break;
                        }
                    }
                }
                Ok(Some(Err(error))) => anyhow::bail!(error),
                Ok(None) | Err(_) => break,
            }
        }
        Ok(())
    }
}

fn serial_reader(
    port: String,
    tx: mpsc::Sender<Result<MochiPacket, String>>,
    stop: Arc<AtomicBool>,
) {
    let mut serial = match serialport::new(&port, SERIAL_BAUD)
        .timeout(StdDuration::from_millis(200))
        .open()
    {
        Ok(port) => port,
        Err(error) => {
            let _ = tx.blocking_send(Err(format!("Could not open {port}: {error}")));
            return;
        }
    };

    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        match read_packet(serial.as_mut(), &stop) {
            Ok(Some(packet)) => {
                if tx.blocking_send(Ok(packet)).is_err() {
                    return;
                }
            }
            Ok(None) => {}
            Err(error) => {
                let _ = tx.blocking_send(Err(error));
                return;
            }
        }
    }
}

fn read_packet(
    serial: &mut dyn serialport::SerialPort,
    stop: &AtomicBool,
) -> Result<Option<MochiPacket>, String> {
    if !find_magic(serial, stop)? {
        return Ok(None);
    }

    let mut header = [0_u8; 3];
    if !read_exact_or_timeout(serial, &mut header, stop)? {
        return Ok(None);
    }
    let packet_type = header[0];
    let payload_len = u16::from_le_bytes([header[1], header[2]]) as usize;
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(format!("Mochi voice packet too large: {payload_len} bytes"));
    }

    let mut payload = vec![0_u8; payload_len];
    if payload_len > 0 && !read_exact_or_timeout(serial, &mut payload, stop)? {
        return Ok(None);
    }

    match packet_type {
        b'S' => Ok(Some(MochiPacket::Start)),
        b'A' => Ok(Some(MochiPacket::Audio(payload))),
        b'E' => Ok(Some(MochiPacket::End)),
        other => Err(format!("Unknown Mochi voice packet type: {other}")),
    }
}

fn find_magic(serial: &mut dyn serialport::SerialPort, stop: &AtomicBool) -> Result<bool, String> {
    let mut window = [0_u8; 4];
    let mut filled = 0_usize;
    let mut byte = [0_u8; 1];

    while !stop.load(Ordering::Relaxed) {
        match serial.read(&mut byte) {
            Ok(1) => {
                if filled < MAGIC.len() {
                    window[filled] = byte[0];
                    filled += 1;
                } else {
                    window.rotate_left(1);
                    window[MAGIC.len() - 1] = byte[0];
                }
                if filled == MAGIC.len() && &window == MAGIC {
                    return Ok(true);
                }
            }
            Ok(_) => {}
            Err(error) if is_read_timeout(&error) => return Ok(false),
            Err(error) => return Err(format!("Serial read failed: {error}")),
        }
    }

    Ok(false)
}

fn read_exact_or_timeout(
    serial: &mut dyn serialport::SerialPort,
    buf: &mut [u8],
    stop: &AtomicBool,
) -> Result<bool, String> {
    let mut offset = 0_usize;
    while offset < buf.len() && !stop.load(Ordering::Relaxed) {
        match serial.read(&mut buf[offset..]) {
            Ok(0) => {}
            Ok(n) => offset += n,
            Err(error) if is_read_timeout(&error) => continue,
            Err(error) => return Err(format!("Serial read failed: {error}")),
        }
    }
    Ok(offset == buf.len())
}

fn is_read_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

fn transcript_from_message(message: &Message) -> Option<(String, bool)> {
    match message {
        Message::Text(text) => transcript_from_json_text(text.as_ref()),
        Message::Binary(bytes) => std::str::from_utf8(bytes)
            .ok()
            .and_then(transcript_from_json_text),
        _ => None,
    }
}

fn transcript_from_json_text(text: &str) -> Option<(String, bool)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let transcript = value
        .get("text")
        .or_else(|| value.pointer("/result/text"))
        .and_then(Value::as_str)?;
    let is_final = value
        .get("is_final")
        .or_else(|| value.get("isFinal"))
        .or_else(|| value.pointer("/result/is_final"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some((transcript.to_string(), is_final))
}

async fn paste_text(text: String) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || paste_text_to_focused_input(&text)).await?
}

fn paste_text_to_focused_input(text: &str) -> anyhow::Result<()> {
    let mut pbcopy = Command::new("pbcopy").stdin(Stdio::piped()).spawn()?;
    if let Some(stdin) = pbcopy.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }
    let status = pbcopy.wait()?;
    if !status.success() {
        anyhow::bail!("pbcopy failed");
    }

    #[cfg(target_os = "macos")]
    {
        let status = Command::new("osascript")
            .arg("-e")
            .arg(r#"tell application "System Events" to keystroke "v" using command down"#)
            .status()?;
        if !status.success() {
            anyhow::bail!("Paste shortcut failed; grant Notice Accessibility permission in macOS Privacy settings");
        }
    }

    #[cfg(not(target_os = "macos"))]
    anyhow::bail!("Mochi voice paste is currently implemented for macOS");

    Ok(())
}

fn same_session_config(left: &MochiVoiceConfig, right: &MochiVoiceConfig) -> bool {
    left.enabled == right.enabled
        && left.serial_port == right.serial_port
        && left.asr_url == right.asr_url
}

fn normalized_or_default(value: Option<&str>, default_value: &str) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_value)
        .to_string()
}

async fn set_status(pool: &SqlitePool, message: &str) -> anyhow::Result<()> {
    storage::put_setting(pool, LAST_STATUS_KEY, message).await
}

#[cfg(test)]
mod tests {
    use super::{normalized_or_default, transcript_from_json_text};

    #[test]
    fn reads_direct_asr_transcript() {
        let result = transcript_from_json_text(r#"{"text":"你好 Mochi","is_final":true}"#);
        assert_eq!(result, Some(("你好 Mochi".to_string(), true)));
    }

    #[test]
    fn reads_nested_asr_transcript() {
        let result = transcript_from_json_text(r#"{"result":{"text":"hello","is_final":false}}"#);
        assert_eq!(result, Some(("hello".to_string(), false)));
    }

    #[test]
    fn normalizes_blank_config_values() {
        assert_eq!(normalized_or_default(Some("  "), "fallback"), "fallback");
        assert_eq!(
            normalized_or_default(Some(" ws://example "), "fallback"),
            "ws://example"
        );
        assert_eq!(normalized_or_default(None, "fallback"), "fallback");
    }
}
