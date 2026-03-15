use anyhow::{Context, Result, bail};
use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use reqwest::blocking::{Client, multipart};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const API_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const MODEL: &str = "whisper-large-v3-turbo";
const DEFAULT_PASTE_SHORTCUTS: [&str; 2] = ["SHIFT, Insert, activewindow", "CTRL, V, activewindow"];
const CLIPBOARD_SETTLE_DELAY: Duration = Duration::from_millis(75);
const RECORD_READY_DELAY: Duration = Duration::from_millis(150);
const STOP_WAIT_DELAY: Duration = Duration::from_millis(50);
const STOP_WAIT_RETRIES: usize = 80;

#[derive(Debug, Deserialize, Serialize)]
struct RecordingState {
    pid: i32,
    audio_path: PathBuf,
    started_at_ms: u128,
}

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("creak: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    load_env();

    match env::args().nth(1).as_deref() {
        None | Some("toggle") => toggle(),
        Some("start") => start_recording(),
        Some("stop") => stop_recording(),
        Some("status") => status(),
        Some(other) => bail!("unknown command: {other}"),
    }
}

fn load_env() {
    if let Some(path) = env::var_os("CREAK_DOTENV") {
        dotenvy::from_path_override(path).ok();
    } else {
        dotenvy::dotenv().ok();
    }
}

fn toggle() -> Result<()> {
    match load_state() {
        Some(state) if process_is_running(state.pid) => stop_recording(),
        Some(_) => {
            cleanup_state().ok();
            start_recording()
        }
        None => start_recording(),
    }
}

fn status() -> Result<()> {
    match load_state() {
        Some(state) if process_is_running(state.pid) => {
            println!("recording {}", state.audio_path.display());
        }
        Some(_) => {
            cleanup_state()?;
            println!("idle");
        }
        None => println!("idle"),
    }

    Ok(())
}

fn start_recording() -> Result<()> {
    if let Some(state) = load_state() {
        if process_is_running(state.pid) {
            bail!("already recording");
        }

        cleanup_state()?;
    }

    let runtime_dir = runtime_dir()?;
    fs::create_dir_all(&runtime_dir)
        .with_context(|| format!("failed to create {}", runtime_dir.display()))?;

    let audio_path = runtime_dir.join(format!("recording-{}.wav", timestamp_ms()));
    let source = detect_source();

    let mut child = Command::new("setsid")
        .arg("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-nostdin")
        .arg("-y")
        .arg("-f")
        .arg("pulse")
        .arg("-i")
        .arg(&source)
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(&audio_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context(
            "failed to start recorder; install setsid, ffmpeg, and ensure PulseAudio/PipeWire is available",
        )?;

    thread::sleep(RECORD_READY_DELAY);
    if let Some(status) = child.try_wait().context("failed to inspect ffmpeg state")? {
        bail!("ffmpeg exited immediately with status {status}");
    }

    let state = RecordingState {
        pid: child.id() as i32,
        audio_path,
        started_at_ms: timestamp_ms(),
    };
    save_state(&state)?;

    println!("recording");
    Ok(())
}

fn stop_recording() -> Result<()> {
    let state = load_state().context("not recording")?;
    cleanup_state()?;

    if process_is_running(state.pid) {
        kill(Pid::from_raw(state.pid), Signal::SIGINT)
            .with_context(|| format!("failed to stop recorder pid {}", state.pid))?;

        wait_for_exit(state.pid)?;
    }

    let transcript = transcribe(&state.audio_path)?;
    insert_text(&transcript)?;

    if let Err(error) = fs::remove_file(&state.audio_path) {
        eprintln!(
            "creak: warning: failed to remove {}: {error}",
            state.audio_path.display()
        );
    }

    println!("{transcript}");
    Ok(())
}

fn transcribe(audio_path: &Path) -> Result<String> {
    let api_key = env::var("GROQ_API_KEY")
        .context("GROQ_API_KEY is missing; keep it in your environment or .env")?;

    let form = multipart::Form::new()
        .text("model", MODEL.to_owned())
        .text("temperature", "0".to_owned())
        .text("response_format", "json".to_owned())
        .part(
            "file",
            multipart::Part::file(audio_path)
                .with_context(|| format!("failed to attach {}", audio_path.display()))?
                .mime_str("audio/wav")
                .context("failed to mark audio as wav")?,
        );

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(90))
        .build()
        .context("failed to build HTTP client")?;

    let response = client
        .post(API_URL)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .context("failed to send transcription request")?
        .error_for_status()
        .context("Groq returned an error response")?;

    let transcription: TranscriptionResponse = response
        .json()
        .context("failed to decode Groq transcription response")?;

    let text = transcription.text.trim().to_owned();
    if text.is_empty() {
        bail!("Groq returned an empty transcript");
    }

    Ok(text)
}

fn insert_text(text: &str) -> Result<()> {
    copy_to_clipboard(text)?;
    thread::sleep(CLIPBOARD_SETTLE_DELAY);

    if env::var("XDG_SESSION_TYPE").ok().as_deref() == Some("wayland") {
        for shortcut in paste_shortcuts() {
            if send_shortcut(&shortcut) {
                return Ok(());
            }
        }
    }

    Ok(())
}

fn paste_shortcuts() -> Vec<String> {
    if let Ok(shortcut) = env::var("CREAK_PASTE_SHORTCUT") {
        let shortcut = shortcut.trim();
        if !shortcut.is_empty() {
            return vec![shortcut.to_owned()];
        }
    }

    DEFAULT_PASTE_SHORTCUTS
        .iter()
        .map(|shortcut| shortcut.to_string())
        .collect()
}

fn send_shortcut(shortcut: &str) -> bool {
    matches!(
        Command::new("hyprctl")
            .arg("dispatch")
            .arg("sendshortcut")
            .arg(shortcut)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
        Ok(status) if status.success()
    )
}

fn copy_to_clipboard(text: &str) -> Result<()> {
    let mut child = Command::new("wl-copy")
        .arg("--type")
        .arg("text/plain;charset=utf-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to start wl-copy")?;

    child
        .stdin
        .as_mut()
        .context("wl-copy stdin was unavailable")?
        .write_all(text.as_bytes())
        .context("failed to write to wl-copy")?;

    let status = child.wait().context("failed to wait for wl-copy")?;
    if !status.success() {
        bail!("wl-copy exited with status {status}");
    }

    Ok(())
}

fn detect_source() -> String {
    env::var("CREAK_SOURCE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(default_source_from_pactl)
        .unwrap_or_else(|| "default".to_owned())
}

fn default_source_from_pactl() -> Option<String> {
    let output = Command::new("pactl")
        .arg("get-default-source")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let source = String::from_utf8(output.stdout).ok()?;
    let source = source.trim();
    if source.is_empty() {
        None
    } else {
        Some(source.to_owned())
    }
}

fn runtime_dir() -> Result<PathBuf> {
    let base = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join(format!("creak-{}", std::process::id())));

    Ok(base.join("creak"))
}

fn state_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("state.json"))
}

fn save_state(state: &RecordingState) -> Result<()> {
    let path = state_path()?;
    let bytes = serde_json::to_vec(state).context("failed to encode recorder state")?;
    fs::write(&path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn load_state() -> Option<RecordingState> {
    let path = state_path().ok()?;
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn cleanup_state() -> Result<()> {
    let path = state_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

fn process_is_running(pid: i32) -> bool {
    matches!(kill(Pid::from_raw(pid), None), Ok(()) | Err(Errno::EPERM))
}

fn wait_for_exit(pid: i32) -> Result<()> {
    for _ in 0..STOP_WAIT_RETRIES {
        if !process_is_running(pid) {
            return Ok(());
        }

        thread::sleep(STOP_WAIT_DELAY);
    }

    bail!("recorder pid {pid} did not exit in time")
}

fn timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
