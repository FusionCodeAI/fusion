//! High-level voice-to-text input handler for the Fusion REPL.
//!
//! Provides [`VoiceInput`], a thin façade over the lower-level [`voice`](super::voice) subsystem
//! that handles backend detection, recording lifecycle, and transcription in one call.
//!
//! # Backend Detection
//!
//! [`VoiceInput::available`] probes the following backends in priority order:
//!
//! 1. **`$WHISPER_PATH`** — explicit path to a `whisper.cpp` binary.
//! 2. **`whisper-cpp`** — whisper.cpp CLI on `$PATH`.
//! 3. **`whisper`** — OpenAI-compatible Whisper CLI on `$PATH`.
//! 4. **macOS `say`** — system speech synthesis (transcription not available; detection only).
//! 5. **`speech-dispatcher` / `spd-say`** — Linux speech daemon (detection only).
//!
//! When none are found, [`VoiceInput::available`] returns `false` and
//! [`VoiceInput::record`] returns [`VoiceInputError::NoBackend`] immediately.
//!
//! # `/voice` Slash Command Flow
//!
//! The intended integration point for the `/voice` slash command:
//!
//! ```text
//! user types /voice
//!   → VoiceInput::new()
//!   → if !available() → display "No voice backend found"
//!   → record() → transcribed text
//!   → paste text into prompt input buffer
//! ```

use std::fmt;
use std::process::Command;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during voice input detection, recording, or transcription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceInputError {
    /// No supported speech-recognition backend is installed or discoverable.
    NoBackend,
    /// The backend binary exists but failed when invoked.
    BackendError(String),
    /// Audio recording produced no usable speech.
    NoSpeech,
    /// Recording or transcription exceeded the configured time limit.
    Timeout,
    /// User or system cancelled the recording session.
    Cancelled,
    /// An I/O error occurred while writing or reading temporary audio files.
    Io(String),
}

impl fmt::Display for VoiceInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VoiceInputError::NoBackend => {
                write!(f, "no voice backend found (install whisper.cpp or enable system speech recognition)")
            }
            VoiceInputError::BackendError(msg) => write!(f, "voice backend error: {}", msg),
            VoiceInputError::NoSpeech => write!(f, "no speech detected in recording"),
            VoiceInputError::Timeout => write!(f, "voice recording timed out"),
            VoiceInputError::Cancelled => write!(f, "voice recording cancelled"),
            VoiceInputError::Io(msg) => write!(f, "voice I/O error: {}", msg),
        }
    }
}

impl std::error::Error for VoiceInputError {}

// ---------------------------------------------------------------------------
// Backend
// ---------------------------------------------------------------------------

/// Detected voice recognition backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceBackend {
    /// A `whisper.cpp`-compatible CLI binary found at the given path.
    WhisperCpp { path: String },
    /// The generic `whisper` CLI (OpenAI open-source) found on `$PATH`.
    WhisperCli { path: String },
    /// macOS `say`/`SFSpeechRecognizer` system backend (available, limited).
    MacOsSystem,
    /// Linux `spd-say` / `speech-dispatcher` backend (available, limited).
    LinuxSpeechDispatcher,
}

impl fmt::Display for VoiceBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VoiceBackend::WhisperCpp { path } => write!(f, "whisper.cpp ({})", path),
            VoiceBackend::WhisperCli { path } => write!(f, "whisper CLI ({})", path),
            VoiceBackend::MacOsSystem => write!(f, "macOS system speech recognition"),
            VoiceBackend::LinuxSpeechDispatcher => write!(f, "Linux speech-dispatcher"),
        }
    }
}

impl VoiceBackend {
    /// Returns `true` if this backend can perform actual speech-to-text transcription
    /// (as opposed to text-to-speech-only system backends).
    pub fn can_transcribe(&self) -> bool {
        matches!(
            self,
            VoiceBackend::WhisperCpp { .. } | VoiceBackend::WhisperCli { .. }
        )
    }

    /// Returns the CLI executable path for invocable backends, or `None` for OS backends.
    pub fn executable(&self) -> Option<&str> {
        match self {
            VoiceBackend::WhisperCpp { path } | VoiceBackend::WhisperCli { path } => {
                Some(path.as_str())
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Checks whether a binary is reachable on `$PATH` (or as an absolute path) by
/// probing it with `--version` or `--help`.  Returns the resolved path string on
/// success.
fn probe_binary(candidate: &str) -> Option<String> {
    // `which`-style resolution: try the candidate as-is (works for absolute paths
    // and bare names that the shell would find on $PATH).
    let output = Command::new(candidate)
        .arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();

    match output {
        Ok(_) => Some(candidate.to_owned()),
        Err(_) => {
            // Try `--version` as a fallback for binaries that don't accept `--help`.
            let output2 = Command::new(candidate)
                .arg("--version")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output();
            output2.ok().map(|_| candidate.to_owned())
        }
    }
}

/// Detects the best available voice backend.  Returns `None` when nothing is found.
fn detect_backend() -> Option<VoiceBackend> {
    // 1. Explicit path from environment variable (whisper.cpp).
    if let Ok(path) = std::env::var("WHISPER_PATH") {
        let trimmed = path.trim().to_owned();
        if !trimmed.is_empty() {
            if let Some(resolved) = probe_binary(&trimmed) {
                return Some(VoiceBackend::WhisperCpp { path: resolved });
            }
        }
    }

    // 2. `whisper-cpp` on $PATH.
    if let Some(p) = probe_binary("whisper-cpp") {
        return Some(VoiceBackend::WhisperCpp { path: p });
    }

    // 3. `whisper` on $PATH (generic OpenAI CLI).
    if let Some(p) = probe_binary("whisper") {
        return Some(VoiceBackend::WhisperCli { path: p });
    }

    // 4. macOS system speech (say is present on every macOS install).
    #[cfg(target_os = "macos")]
    if probe_binary("say").is_some() {
        return Some(VoiceBackend::MacOsSystem);
    }

    // 5. Linux speech-dispatcher.
    #[cfg(target_os = "linux")]
    if probe_binary("spd-say").is_some() {
        return Some(VoiceBackend::LinuxSpeechDispatcher);
    }

    None
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for a voice input session.
#[derive(Debug, Clone)]
pub struct VoiceInputConfig {
    /// Maximum recording duration before auto-stop.
    pub max_duration: Duration,
    /// Whisper model identifier passed to CLI backends (e.g. `"base.en"`, `"small"`).
    pub model: String,
    /// Optional BCP-47 language hint (e.g. `"en"`, `"ja"`).  `None` enables auto-detect.
    pub language: Option<String>,
}

impl Default for VoiceInputConfig {
    fn default() -> Self {
        Self {
            max_duration: Duration::from_secs(60),
            model: std::env::var("WHISPER_MODEL")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "base.en".to_owned()),
            language: std::env::var("VOICE_LANGUAGE")
                .ok()
                .filter(|s| !s.trim().is_empty()),
        }
    }
}

// ---------------------------------------------------------------------------
// VoiceInput — main public API
// ---------------------------------------------------------------------------

/// High-level voice-to-text input handler.
///
/// # Example
///
/// ```rust,no_run
/// use fusion::ui::voice_input::VoiceInput;
///
/// fn handle_voice_slash_command(input_buffer: &mut String) {
///     let vi = VoiceInput::new();
///     if !vi.available() {
///         eprintln!("No voice backend found.");
///         return;
///     }
///     match vi.record() {
///         Ok(text) => {
///             input_buffer.push_str(&text);
///         }
///         Err(e) => eprintln!("Voice error: {}", e),
///     }
/// }
/// ```
pub struct VoiceInput {
    backend: Option<VoiceBackend>,
    config: VoiceInputConfig,
}

impl VoiceInput {
    /// Creates a new `VoiceInput`, probing for available backends immediately.
    pub fn new() -> Self {
        Self {
            backend: detect_backend(),
            config: VoiceInputConfig::default(),
        }
    }

    /// Creates a `VoiceInput` with a custom configuration.
    pub fn with_config(config: VoiceInputConfig) -> Self {
        Self {
            backend: detect_backend(),
            config,
        }
    }

    /// Returns `true` when at least one speech recognition backend is available.
    ///
    /// Always returns `false` gracefully when no backend is installed; never panics.
    pub fn available(&self) -> bool {
        self.backend
            .as_ref()
            .map(|b| b.can_transcribe())
            .unwrap_or(false)
    }

    /// Returns the detected backend, if any.
    pub fn backend(&self) -> Option<&VoiceBackend> {
        self.backend.as_ref()
    }

    /// Returns a human-readable description of the detected backend, suitable for
    /// display in the terminal UI.
    pub fn backend_label(&self) -> &str {
        match &self.backend {
            None => "none",
            Some(VoiceBackend::WhisperCpp { .. }) => "whisper.cpp",
            Some(VoiceBackend::WhisperCli { .. }) => "whisper CLI",
            Some(VoiceBackend::MacOsSystem) => "macOS system",
            Some(VoiceBackend::LinuxSpeechDispatcher) => "speech-dispatcher",
        }
    }

    /// Records audio from the default input device, transcribes it, and returns
    /// the resulting text ready for pasting into the prompt input buffer.
    ///
    /// Returns [`VoiceInputError::NoBackend`] immediately when no transcription-capable
    /// backend is available — this path is always safe and never blocks.
    ///
    /// # Implementation note
    ///
    /// Audio capture without a native crate (e.g. `cpal`) requires spawning an
    /// external recorder such as `sox` / `arecord` / `ffmpeg`.  This implementation
    /// drives the detected Whisper CLI backend end-to-end:
    ///
    /// 1. Record WAV via `sox` or `arecord` into a temporary file (with timeout).
    /// 2. Pass the file to the Whisper binary for transcription.
    /// 3. Return the trimmed output text.
    ///
    /// When neither `sox` nor `arecord` is available the method returns
    /// [`VoiceInputError::BackendError`] with a clear message.
    pub fn record(&self) -> Result<String, VoiceInputError> {
        let backend = match &self.backend {
            Some(b) if b.can_transcribe() => b,
            Some(_) => {
                return Err(VoiceInputError::BackendError(
                    "detected backend does not support transcription".to_owned(),
                ))
            }
            None => return Err(VoiceInputError::NoBackend),
        };

        // Locate an audio capture helper.
        let recorder = detect_audio_recorder().ok_or_else(|| {
            VoiceInputError::BackendError(
                "no audio recorder found; install sox, arecord (Linux), or rec (macOS)".to_owned(),
            )
        })?;

        // Write to a temp file.
        let tmp_dir =
            std::env::temp_dir().join(format!("fusion_voice_{}", timestamp_nanos()));
        std::fs::create_dir_all(&tmp_dir).map_err(|e| VoiceInputError::Io(e.to_string()))?;
        let audio_path = tmp_dir.join("input.wav");

        // Record audio.
        let record_status = recorder
            .record(&audio_path, self.config.max_duration)
            .map_err(|e| VoiceInputError::BackendError(e))?;

        if !record_status {
            return Err(VoiceInputError::Cancelled);
        }

        if !audio_path.exists() || audio_path.metadata().map(|m| m.len()).unwrap_or(0) < 1024 {
            return Err(VoiceInputError::NoSpeech);
        }

        // Transcribe.
        let text = transcribe_with_backend(backend, &audio_path, &self.config)
            .map_err(|e| VoiceInputError::BackendError(e))?;

        // Cleanup temp dir (best-effort).
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let trimmed = text.trim().to_owned();
        if trimmed.is_empty() {
            return Err(VoiceInputError::NoSpeech);
        }

        Ok(trimmed)
    }
}

impl Default for VoiceInput {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for VoiceInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VoiceInput")
            .field("backend", &self.backend)
            .field("available", &self.available())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Audio recorder abstraction
// ---------------------------------------------------------------------------

/// Lightweight audio recorder that drives an external CLI tool.
enum AudioRecorder {
    Sox { path: String },
    Arecord { path: String },
    Rec { path: String },
}

impl AudioRecorder {
    /// Records to `output_path` for at most `max_duration`.
    /// Returns `Ok(true)` on normal exit, `Ok(false)` on user cancel (SIGINT),
    /// `Err(String)` on hard failure.
    fn record(
        &self,
        output_path: &std::path::Path,
        max_duration: Duration,
    ) -> Result<bool, String> {
        let secs = max_duration.as_secs().max(1);
        let path_str = output_path
            .to_str()
            .ok_or_else(|| "non-UTF-8 temp path".to_owned())?;

        let mut cmd = match self {
            AudioRecorder::Sox { path } | AudioRecorder::Rec { path } => {
                let mut c = Command::new(path);
                // sox/rec: input from default mic, output as 16-bit 16 kHz mono WAV.
                c.args([
                    "-d",        // default input device
                    "-r", "16000",
                    "-c", "1",
                    "-b", "16",
                    "-e", "signed-integer",
                    path_str,
                    "trim", "0", &secs.to_string(),
                ]);
                c
            }
            AudioRecorder::Arecord { path } => {
                let mut c = Command::new(path);
                c.args([
                    "-f", "S16_LE",
                    "-r", "16000",
                    "-c", "1",
                    "-d", &secs.to_string(),
                    path_str,
                ]);
                c
            }
        };

        let status = cmd
            .status()
            .map_err(|e| format!("failed to spawn audio recorder: {}", e))?;

        Ok(status.success())
    }
}

/// Probes for an available audio recorder CLI and returns the first found.
fn detect_audio_recorder() -> Option<AudioRecorder> {
    if let Some(p) = probe_binary("sox") {
        return Some(AudioRecorder::Sox { path: p });
    }
    // `rec` is part of the SoX distribution (same binary, different invocation name).
    if let Some(p) = probe_binary("rec") {
        return Some(AudioRecorder::Rec { path: p });
    }
    if let Some(p) = probe_binary("arecord") {
        return Some(AudioRecorder::Arecord { path: p });
    }
    None
}

// ---------------------------------------------------------------------------
// Transcription
// ---------------------------------------------------------------------------

/// Runs the whisper CLI on `audio_path` and returns the transcript text.
fn transcribe_with_backend(
    backend: &VoiceBackend,
    audio_path: &std::path::Path,
    config: &VoiceInputConfig,
) -> Result<String, String> {
    let exe = backend
        .executable()
        .ok_or_else(|| "backend has no executable".to_owned())?;

    let audio_str = audio_path
        .to_str()
        .ok_or_else(|| "non-UTF-8 audio path".to_owned())?;

    let output_dir = audio_path
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".");

    let mut cmd = Command::new(exe);
    cmd.arg(audio_str)
        .arg("--model")
        .arg(&config.model)
        .arg("--output_format")
        .arg("txt")
        .arg("--output_dir")
        .arg(output_dir);

    if let Some(lang) = &config.language {
        cmd.arg("--language").arg(lang);
    }

    // Suppress verbose progress output; whisper writes it to stderr.
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());

    let start = Instant::now();
    let output = cmd
        .output()
        .map_err(|e| format!("failed to spawn whisper: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "whisper exited with non-zero status after {:.1}s",
            start.elapsed().as_secs_f32()
        ));
    }

    // Whisper writes to `<output_dir>/<stem>.txt`.  Try that first, then fall
    // back to stdout (some whisper builds print to stdout with `--output_format txt`).
    let txt_path = audio_path
        .file_stem()
        .map(|s| {
            audio_path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join(format!("{}.txt", s.to_string_lossy()))
        });

    let text = if let Some(p) = txt_path.filter(|p| p.exists()) {
        std::fs::read_to_string(&p).unwrap_or_default()
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };

    Ok(text)
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

fn timestamp_nanos() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_returns_false_gracefully_when_no_backend() {
        // Construct directly with backend = None; must not panic and must return false.
        let vi = VoiceInput {
            backend: None,
            config: VoiceInputConfig::default(),
        };
        assert!(!vi.available());
    }

    #[test]
    fn record_returns_no_backend_error_when_unavailable() {
        let vi = VoiceInput {
            backend: None,
            config: VoiceInputConfig::default(),
        };
        assert_eq!(vi.record(), Err(VoiceInputError::NoBackend));
    }

    #[test]
    fn backend_label_none() {
        let vi = VoiceInput {
            backend: None,
            config: VoiceInputConfig::default(),
        };
        assert_eq!(vi.backend_label(), "none");
    }

    #[test]
    fn backend_label_whisper_cpp() {
        let vi = VoiceInput {
            backend: Some(VoiceBackend::WhisperCpp {
                path: "/usr/local/bin/whisper-cpp".to_owned(),
            }),
            config: VoiceInputConfig::default(),
        };
        assert_eq!(vi.backend_label(), "whisper.cpp");
        assert!(vi.available());
    }

    #[test]
    fn backend_label_macos_system() {
        let vi = VoiceInput {
            backend: Some(VoiceBackend::MacOsSystem),
            config: VoiceInputConfig::default(),
        };
        assert_eq!(vi.backend_label(), "macOS system");
        // macOS system backend cannot transcribe; available() must return false.
        assert!(!vi.available());
    }

    #[test]
    fn record_errors_on_non_transcribing_backend() {
        let vi = VoiceInput {
            backend: Some(VoiceBackend::MacOsSystem),
            config: VoiceInputConfig::default(),
        };
        match vi.record() {
            Err(VoiceInputError::BackendError(_)) => {}
            other => panic!("expected BackendError, got {:?}", other),
        }
    }

    #[test]
    fn voice_input_error_display() {
        assert!(VoiceInputError::NoBackend.to_string().contains("no voice backend found"));
        assert!(VoiceInputError::NoSpeech.to_string().contains("no speech"));
        assert!(VoiceInputError::Timeout.to_string().contains("timed out"));
        assert!(VoiceInputError::Cancelled.to_string().contains("cancelled"));
        assert!(
            VoiceInputError::BackendError("boom".to_owned())
                .to_string()
                .contains("boom")
        );
        assert!(
            VoiceInputError::Io("disk full".to_owned())
                .to_string()
                .contains("disk full")
        );
    }

    #[test]
    fn voice_backend_can_transcribe() {
        assert!(VoiceBackend::WhisperCpp { path: "/usr/bin/whisper-cpp".into() }.can_transcribe());
        assert!(VoiceBackend::WhisperCli { path: "/usr/bin/whisper".into() }.can_transcribe());
        assert!(!VoiceBackend::MacOsSystem.can_transcribe());
        assert!(!VoiceBackend::LinuxSpeechDispatcher.can_transcribe());
    }

    #[test]
    fn voice_input_config_default_uses_env() {
        // Smoke test: default config must not panic and must set a non-empty model.
        let cfg = VoiceInputConfig::default();
        assert!(!cfg.model.is_empty());
        assert!(cfg.max_duration.as_secs() > 0);
    }

    #[test]
    fn debug_impl_does_not_panic() {
        let vi = VoiceInput::new();
        let _ = format!("{:?}", vi);
    }

    #[test]
    fn new_does_not_panic() {
        // Detection may or may not find a backend; either outcome is valid.
        let vi = VoiceInput::new();
        // available() is always well-defined.
        let _ = vi.available();
    }
}
