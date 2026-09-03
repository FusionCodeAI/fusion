//! Voice feedback, speech-to-text (STT) input, and text-to-speech (TTS) assistant integration for Fusion.
//!
//! Provides a pure Rust, zero-C-dependency audio subsystem for interactive voice workflows:
//! - Audio format definitions and in-memory sample buffering (`AudioBuffer`, `AudioFormat`).
//! - Pure Rust RIFF WAVE serializer and deserializer (no external C/C++ libraries).
//! - Voice Activity Detection (VAD) and audio level / dBFS metering for terminal visualization.
//! - Speech-to-text (STT) transcription abstraction via the [`SpeechToTextAdapter`] trait.
//! - Built-in adapters: OpenAI Whisper, Groq Whisper, Custom HTTP STT, Local CLI Whisper, and Mock.
//! - High-level voice input state machine and recording session (`VoiceSession`, `VoiceInputState`).
//! - Text-to-speech announcement dispatcher using native platform binaries (`say` on macOS, `spd-say` on Linux, PowerShell on Windows).
//! - Audio alert chimes for task completion, advisor warnings, error alerts, and voice session cues.
//! - Configurable speech rate, voice selection, volume, and global mute toggle.
//! - Unified [`VoiceAssistant`] coordinator connecting STT input, TTS output, and procedural audio chimes.
//! - Terminal UI helpers: voice badges, audio meters, and waveform sparklines.
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Error Types
// ---------------------------------------------------------------------------

/// Errors that can occur during audio capture, encoding, or speech-to-text transcription.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum VoiceError {
    /// Audio input or recording hardware device error.
    #[error("audio input device error: {0}")]
    DeviceError(String),

    /// Error encoding or decoding audio data (e.g. invalid WAV header).
    #[error("audio encoding error: {0}")]
    EncodingError(String),

    /// HTTP client or transport network error.
    #[error("network error during transcription: {0}")]
    NetworkError(String),

    /// Remote STT API returned an error response.
    #[error("speech-to-text API error (status {status}): {message}")]
    ApiError { status: u16, message: String },

    /// Authentication error (e.g. missing or invalid API key).
    #[error("authentication error: {0}")]
    AuthError(String),

    /// No speech was detected in the provided audio recording.
    #[error("no speech detected in audio input")]
    NoSpeechDetected,

    /// Recording or transcription timed out.
    #[error("voice operation timed out: {0}")]
    Timeout(String),

    /// Voice input or the requested STT provider is not configured.
    #[error("voice input not configured: {0}")]
    NotConfigured(String),

    /// Requested audio format is not supported by the adapter.
    #[error("unsupported audio format: {0}")]
    UnsupportedFormat(String),

    /// Local CLI process execution error.
    #[error("local STT command failed: {0}")]
    ProcessError(String),
    /// Operation was cancelled by user.
    #[error("voice input cancelled")]
    Cancelled,

    /// Text-to-speech engine or binary execution error.
    #[error("text-to-speech error: {0}")]
    TtsError(String),

    /// Voice audio output or audio alert chime is muted.
    #[error("voice audio output is muted")]
    Muted,

    /// Platform not supported for native text-to-speech.
    #[error("TTS platform not supported: {0}")]
    UnsupportedPlatform(String),

    /// Audio chime generation or playback error.
    #[error("audio chime playback error: {0}")]
    ChimeError(String),
}

// ---------------------------------------------------------------------------
// Audio Formats & Constants
// ---------------------------------------------------------------------------

/// Standard sample rate for speech recognition (16 kHz is Whisper's native rate).
pub const DEFAULT_SAMPLE_RATE: u32 = 16_000;

/// Standard channel count for speech input (mono).
pub const DEFAULT_CHANNELS: u16 = 1;

/// Standard bit depth for PCM audio (16-bit signed integer).
pub const DEFAULT_BITS_PER_SAMPLE: u16 = 16;

/// Default silence threshold in dBFS (Decibels relative to Full Scale).
pub const DEFAULT_SILENCE_THRESHOLD_DB: f32 = -42.0;

/// Default silence duration after speech to trigger auto-stop (milliseconds).
pub const DEFAULT_SILENCE_TIMEOUT_MS: u64 = 1_500;

/// Default maximum recording duration in seconds.
pub const DEFAULT_MAX_RECORDING_SECS: u32 = 60;

/// Supported audio container and compression formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    /// Standard uncompressed RIFF WAVE container (PCM 16-bit).
    Wav,
    /// Raw uncompressed PCM 16-bit little-endian samples.
    Pcm16Le,
    /// MPEG-1 Audio Layer III.
    Mp3,
    /// Ogg Vorbis/Opus.
    Ogg,
    /// Free Lossless Audio Codec.
    Flac,
    /// WebM container (Opus audio).
    Webm,
    /// MPEG-4 Audio (AAC/ALAC).
    M4a,
}

impl AudioFormat {
    /// Returns the MIME content-type associated with this audio format.
    pub fn mime_type(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "audio/wav",
            AudioFormat::Pcm16Le => "audio/l16",
            AudioFormat::Mp3 => "audio/mpeg",
            AudioFormat::Ogg => "audio/ogg",
            AudioFormat::Flac => "audio/flac",
            AudioFormat::Webm => "audio/webm",
            AudioFormat::M4a => "audio/m4a",
        }
    }

    /// Returns the standard file extension without leading dot.
    pub fn file_extension(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Pcm16Le => "raw",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Ogg => "ogg",
            AudioFormat::Flac => "flac",
            AudioFormat::Webm => "webm",
            AudioFormat::M4a => "m4a",
        }
    }

    /// Attempts to parse an audio format from a file extension or MIME type.
    pub fn from_mime_or_ext(s: &str) -> Option<Self> {
        let cleaned = s.trim().to_lowercase();
        let cleaned = cleaned.trim_start_matches('.');
        match cleaned {
            "wav" | "wave" | "audio/wav" | "audio/x-wav" | "audio/wave" => Some(AudioFormat::Wav),
            "pcm" | "raw" | "l16" | "audio/l16" | "audio/pcm" => Some(AudioFormat::Pcm16Le),
            "mp3" | "audio/mp3" | "audio/mpeg" => Some(AudioFormat::Mp3),
            "ogg" | "oga" | "opus" | "audio/ogg" | "audio/opus" => Some(AudioFormat::Ogg),
            "flac" | "audio/flac" | "audio/x-flac" => Some(AudioFormat::Flac),
            "webm" | "audio/webm" => Some(AudioFormat::Webm),
            "m4a" | "aac" | "audio/m4a" | "audio/mp4" | "audio/aac" => Some(AudioFormat::M4a),
            _ => None,
        }
    }
}

impl Default for AudioFormat {
    fn default() -> Self {
        AudioFormat::Wav
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file_extension())
    }
}

// ---------------------------------------------------------------------------
// Audio Buffer & Pure Rust RIFF WAVE Processing
// ---------------------------------------------------------------------------

/// In-memory audio sample buffer holding 16-bit linear PCM audio samples.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioBuffer {
    /// 16-bit signed integer linear PCM samples.
    pub samples: Vec<i16>,
    /// Sampling rate in Hz (e.g. 16000).
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo).
    pub channels: u16,
}

impl AudioBuffer {
    /// Creates a new empty audio buffer with the specified sample rate and channel count.
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            samples: Vec::new(),
            sample_rate: if sample_rate == 0 {
                DEFAULT_SAMPLE_RATE
            } else {
                sample_rate
            },
            channels: if channels == 0 {
                DEFAULT_CHANNELS
            } else {
                channels
            },
        }
    }

    /// Creates a new empty audio buffer with default voice settings (16 kHz, mono).
    pub fn new_voice() -> Self {
        Self::new(DEFAULT_SAMPLE_RATE, DEFAULT_CHANNELS)
    }

    /// Creates an audio buffer with pre-allocated sample capacity.
    pub fn with_capacity(capacity: usize, sample_rate: u32, channels: u16) -> Self {
        Self {
            samples: Vec::with_capacity(capacity),
            sample_rate: if sample_rate == 0 {
                DEFAULT_SAMPLE_RATE
            } else {
                sample_rate
            },
            channels: if channels == 0 {
                DEFAULT_CHANNELS
            } else {
                channels
            },
        }
    }

    /// Creates an audio buffer from raw 16-bit signed PCM samples.
    pub fn from_i16_samples(samples: Vec<i16>, sample_rate: u32, channels: u16) -> Self {
        Self {
            samples,
            sample_rate: if sample_rate == 0 {
                DEFAULT_SAMPLE_RATE
            } else {
                sample_rate
            },
            channels: if channels == 0 {
                DEFAULT_CHANNELS
            } else {
                channels
            },
        }
    }

    /// Creates an audio buffer from normalized 32-bit floating point samples (-1.0 to 1.0).
    pub fn from_f32_samples(samples: &[f32], sample_rate: u32, channels: u16) -> Self {
        let i16_samples = samples
            .iter()
            .map(|&s| {
                let clamped = s.clamp(-1.0, 1.0);
                if clamped >= 0.0 {
                    (clamped * 32767.0) as i16
                } else {
                    (clamped * 32768.0) as i16
                }
            })
            .collect();

        Self::from_i16_samples(i16_samples, sample_rate, channels)
    }

    /// Creates an audio buffer from raw little-endian PCM16 byte slice.
    pub fn from_pcm16_le(
        bytes: &[u8],
        sample_rate: u32,
        channels: u16,
    ) -> Result<Self, VoiceError> {
        if bytes.len() % 2 != 0 {
            return Err(VoiceError::EncodingError(
                "PCM 16-bit byte stream length must be even".to_string(),
            ));
        }

        let mut samples = Vec::with_capacity(bytes.len() / 2);
        for chunk in bytes.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            samples.push(sample);
        }

        Ok(Self::from_i16_samples(samples, sample_rate, channels))
    }

    /// Appends a single PCM sample.
    pub fn push_sample(&mut self, sample: i16) {
        self.samples.push(sample);
    }

    /// Appends a slice of 16-bit PCM samples.
    pub fn push_samples(&mut self, samples: &[i16]) {
        self.samples.extend_from_slice(samples);
    }

    /// Appends a slice of normalized 32-bit floating point samples (-1.0 to 1.0).
    pub fn push_f32_samples(&mut self, samples: &[f32]) {
        for &s in samples {
            let clamped = s.clamp(-1.0, 1.0);
            let sample_i16 = if clamped >= 0.0 {
                (clamped * 32767.0) as i16
            } else {
                (clamped * 32768.0) as i16
            };
            self.samples.push(sample_i16);
        }
    }

    /// Appends raw little-endian PCM16 bytes.
    pub fn extend_pcm16_le(&mut self, bytes: &[u8]) -> Result<usize, VoiceError> {
        if bytes.len() % 2 != 0 {
            return Err(VoiceError::EncodingError(
                "PCM 16-bit byte stream length must be even".to_string(),
            ));
        }
        let count = bytes.len() / 2;
        self.samples.reserve(count);
        for chunk in bytes.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            self.samples.push(sample);
        }
        Ok(count)
    }

    /// Returns the number of individual audio samples.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Returns true if the buffer has zero samples.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Clears all samples in the buffer while retaining allocated capacity.
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Returns the total duration of the buffered audio in seconds.
    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 || self.channels == 0 {
            return 0.0;
        }
        let total_frames = self.samples.len() as f64 / self.channels as f64;
        total_frames / self.sample_rate as f64
    }

    /// Returns the total duration of the buffered audio in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        (self.duration_secs() * 1000.0).round() as u64
    }

    /// Computes the Root Mean Square (RMS) amplitude of all samples in the buffer (normalized 0.0 to 1.0).
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f64 = self
            .samples
            .iter()
            .map(|&s| {
                let norm = s as f64 / 32768.0;
                norm * norm
            })
            .sum();
        let mean_sq = sum_sq / (self.samples.len() as f64);
        mean_sq.sqrt() as f32
    }

    /// Computes the peak absolute amplitude in the buffer (normalized 0.0 to 1.0).
    pub fn peak(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let max_abs = self
            .samples
            .iter()
            .map(|&s| s.abs() as u32)
            .max()
            .unwrap_or(0);
        (max_abs as f32) / 32768.0
    }

    /// Computes the RMS volume level in Decibels relative to Full Scale (dBFS).
    /// Returns a value between -96.0 dBFS (virtual silence) and 0.0 dBFS (maximum loudness).
    pub fn rms_db(&self) -> f32 {
        let rms_val = self.rms();
        if rms_val <= 1e-5 {
            -96.0
        } else {
            let db = 20.0 * rms_val.log10();
            db.max(-96.0).min(0.0)
        }
    }

    /// Returns true if the buffer's RMS level is strictly below the specified silence threshold in dBFS.
    pub fn is_silent(&self, threshold_db: f32) -> bool {
        self.rms_db() < threshold_db
    }

    /// Serializes the audio buffer into raw 16-bit little-endian PCM bytes.
    pub fn to_pcm16_le_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.samples.len() * 2);
        for &sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    /// Encodes the PCM audio buffer into a valid, standard RIFF WAVE file binary buffer in pure Rust.
    ///
    /// Generates a standard 44-byte canonical WAV header:
    /// - RIFF chunk header
    /// - `fmt ` subchunk (AudioFormat 1 = PCM, 16 bits per sample)
    /// - `data` subchunk containing little-endian PCM samples
    pub fn to_wav_bytes(&self) -> Vec<u8> {
        let bits_per_sample = DEFAULT_BITS_PER_SAMPLE;
        let channels = self.channels;
        let sample_rate = self.sample_rate;
        let byte_rate = sample_rate * (channels as u32) * (bits_per_sample as u32) / 8;
        let block_align = channels * bits_per_sample / 8;
        let data_size = (self.samples.len() * 2) as u32;
        let total_riff_size = 36 + data_size;

        let mut wav = Vec::with_capacity(44 + data_size as usize);

        // 1. RIFF header
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&total_riff_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        // 2. "fmt " subchunk (16 bytes for standard PCM)
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // Subchunk1Size = 16
        wav.extend_from_slice(&1u16.to_le_bytes()); // AudioFormat = 1 (PCM)
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());

        // 3. "data" subchunk
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());

        // 4. PCM 16-bit little-endian sample payload
        for &sample in &self.samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }

        wav
    }

    /// Parses a standard RIFF WAVE binary file in pure Rust into an [`AudioBuffer`].
    pub fn from_wav_bytes(data: &[u8]) -> Result<Self, VoiceError> {
        if data.len() < 44 {
            return Err(VoiceError::EncodingError(
                "WAV file data too short (minimum header size is 44 bytes)".to_string(),
            ));
        }

        // Validate RIFF header
        if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
            return Err(VoiceError::EncodingError(
                "Invalid RIFF WAVE file header magic".to_string(),
            ));
        }

        let mut offset = 12;
        let mut sample_rate = DEFAULT_SAMPLE_RATE;
        let mut channels = DEFAULT_CHANNELS;
        let mut bits_per_sample = DEFAULT_BITS_PER_SAMPLE;
        let mut audio_format = 1u16;
        let mut data_slice: Option<&[u8]> = None;

        while offset + 8 <= data.len() {
            let chunk_id = &data[offset..offset + 4];
            let chunk_size = u32::from_le_bytes([
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]) as usize;
            offset += 8;

            if offset + chunk_size > data.len() {
                // If chunk size extends beyond buffer, truncate to available length
                let remaining = data.len().saturating_sub(offset);
                if chunk_id == b"data" {
                    data_slice = Some(&data[offset..offset + remaining]);
                }
                break;
            }

            if chunk_id == b"fmt " {
                if chunk_size < 16 {
                    return Err(VoiceError::EncodingError(
                        "WAV 'fmt ' chunk size too small".to_string(),
                    ));
                }
                audio_format = u16::from_le_bytes([data[offset], data[offset + 1]]);
                channels = u16::from_le_bytes([data[offset + 2], data[offset + 3]]);
                sample_rate = u32::from_le_bytes([
                    data[offset + 4],
                    data[offset + 5],
                    data[offset + 6],
                    data[offset + 7],
                ]);
                bits_per_sample = u16::from_le_bytes([data[offset + 14], data[offset + 15]]);
            } else if chunk_id == b"data" {
                data_slice = Some(&data[offset..offset + chunk_size]);
            }

            // Word-align chunk offset
            offset += chunk_size;
            if chunk_size % 2 != 0 {
                offset += 1;
            }
        }

        if audio_format != 1 {
            return Err(VoiceError::EncodingError(format!(
                "Unsupported WAV audio format tag {} (only uncompressed PCM = 1 supported)",
                audio_format
            )));
        }

        if bits_per_sample != 16 {
            return Err(VoiceError::EncodingError(format!(
                "Unsupported bit depth {} bits (only 16-bit PCM supported)",
                bits_per_sample
            )));
        }

        let raw_data = data_slice.ok_or_else(|| {
            VoiceError::EncodingError("Missing 'data' subchunk in WAV file".to_string())
        })?;

        Self::from_pcm16_le(raw_data, sample_rate, channels)
    }

    /// Generates a test audio buffer containing a pure sinusoidal tone.
    pub fn generate_sine_wave(
        freq_hz: f32,
        duration_secs: f32,
        sample_rate: u32,
        amplitude: f32,
    ) -> Self {
        let total_samples = (sample_rate as f32 * duration_secs).round() as usize;
        let mut samples = Vec::with_capacity(total_samples);
        let amp = amplitude.clamp(0.0, 1.0);

        for i in 0..total_samples {
            let t = i as f32 / sample_rate as f32;
            let val = (2.0 * std::f32::consts::PI * freq_hz * t).sin() * amp;
            let sample_i16 = (val * 32767.0) as i16;
            samples.push(sample_i16);
        }

        Self::from_i16_samples(samples, sample_rate, 1)
    }

    /// Generates a test audio buffer containing silence.
    pub fn generate_silence(duration_secs: f32, sample_rate: u32) -> Self {
        let total_samples = (sample_rate as f32 * duration_secs).round() as usize;
        let samples = vec![0i16; total_samples];
        Self::from_i16_samples(samples, sample_rate, 1)
    }

    /// Appends all samples from another audio buffer.
    pub fn append_buffer(&mut self, other: &AudioBuffer) {
        self.samples.extend_from_slice(&other.samples);
    }

    /// Applies a smooth attack and decay envelope in-place to avoid audio clicks at chunk boundaries.
    pub fn apply_envelope(&mut self, attack_samples: usize, decay_samples: usize) {
        let n = self.samples.len();
        if n == 0 {
            return;
        }

        let attack = attack_samples.min(n / 2);
        if attack > 0 {
            for i in 0..attack {
                let factor = i as f32 / attack as f32;
                let val = (self.samples[i] as f32 * factor).round();
                self.samples[i] = val.clamp(-32768.0, 32767.0) as i16;
            }
        }

        let decay = decay_samples.min(n / 2);
        if decay > 0 {
            let decay_start = n.saturating_sub(decay);
            for i in decay_start..n {
                let remaining = n - 1 - i;
                let factor = remaining as f32 / decay as f32;
                let val = (self.samples[i] as f32 * factor).round();
                self.samples[i] = val.clamp(-32768.0, 32767.0) as i16;
            }
        }
    }

    /// Generates a smooth sinusoidal tone with configurable attack and decay envelopes.
    pub fn generate_tone_with_envelope(
        freq_hz: f32,
        duration_secs: f32,
        sample_rate: u32,
        amplitude: f32,
        attack_secs: f32,
        decay_secs: f32,
    ) -> Self {
        let mut buf = Self::generate_sine_wave(freq_hz, duration_secs, sample_rate, amplitude);
        let attack_samples = (sample_rate as f32 * attack_secs).round() as usize;
        let decay_samples = (sample_rate as f32 * decay_secs).round() as usize;
        buf.apply_envelope(attack_samples, decay_samples);
        buf
    }
}

impl Default for AudioBuffer {
    fn default() -> Self {
        Self::new_voice()
    }
}

// ---------------------------------------------------------------------------
// Voice Activity Detection (VAD) & Audio Level Metering
// ---------------------------------------------------------------------------

/// Configuration for the Voice Activity Detection (VAD) state machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VadConfig {
    /// Silence threshold in dBFS below which audio is classified as silence.
    pub threshold_db: f32,
    /// Duration of continuous silence (in ms) after speech has started that triggers speech completion.
    pub silence_timeout_ms: u64,
    /// Minimum speech duration (in ms) before registering that voice activity actually started.
    pub min_speech_duration_ms: u64,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            threshold_db: DEFAULT_SILENCE_THRESHOLD_DB,
            silence_timeout_ms: DEFAULT_SILENCE_TIMEOUT_MS,
            min_speech_duration_ms: 250,
        }
    }
}

/// State of voice activity detection for a recording stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VadState {
    /// Listening for incoming voice activity; currently silent.
    Listening,
    /// User is actively speaking.
    Speaking,
    /// User was speaking and has now ceased for longer than the silence timeout.
    SpeechEnded,
}

impl fmt::Display for VadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VadState::Listening => write!(f, "listening"),
            VadState::Speaking => write!(f, "speaking"),
            VadState::SpeechEnded => write!(f, "speech_ended"),
        }
    }
}

/// Real-time Voice Activity Detector that monitors incoming PCM audio frames.
#[derive(Debug, Clone)]
pub struct VadDetector {
    config: VadConfig,
    state: VadState,
    speech_detected: bool,
    accumulated_speech_ms: u64,
    accumulated_silence_ms: u64,
}

impl VadDetector {
    /// Creates a new VAD detector with the specified configuration.
    pub fn new(config: VadConfig) -> Self {
        Self {
            config,
            state: VadState::Listening,
            speech_detected: false,
            accumulated_speech_ms: 0,
            accumulated_silence_ms: 0,
        }
    }

    /// Resets the VAD detector state to initial listening mode.
    pub fn reset(&mut self) {
        self.state = VadState::Listening;
        self.speech_detected = false;
        self.accumulated_speech_ms = 0;
        self.accumulated_silence_ms = 0;
    }

    /// Current VAD state.
    pub fn state(&self) -> VadState {
        self.state
    }

    /// Whether speech has been detected at any point in this session.
    pub fn has_speech_started(&self) -> bool {
        self.speech_detected
    }

    /// Processes an incoming audio buffer slice and updates internal state.
    pub fn process_chunk(&mut self, chunk: &AudioBuffer) -> VadState {
        if chunk.is_empty() {
            return self.state;
        }

        let chunk_ms = chunk.duration_ms();
        let is_silent = chunk.is_silent(self.config.threshold_db);

        if !is_silent {
            // Audio level is active
            self.accumulated_speech_ms += chunk_ms;
            self.accumulated_silence_ms = 0;

            if !self.speech_detected
                && self.accumulated_speech_ms >= self.config.min_speech_duration_ms
            {
                self.speech_detected = true;
                self.state = VadState::Speaking;
            } else if self.speech_detected {
                self.state = VadState::Speaking;
            }
        } else {
            // Audio level is silent
            if self.speech_detected {
                self.accumulated_silence_ms += chunk_ms;
                if self.accumulated_silence_ms >= self.config.silence_timeout_ms {
                    self.state = VadState::SpeechEnded;
                }
            } else {
                self.accumulated_speech_ms = 0;
                self.state = VadState::Listening;
            }
        }

        self.state
    }
}

/// Helper for rendering audio volume levels and VU meters in terminal interfaces.
pub struct AudioLevelMeter;

impl AudioLevelMeter {
    /// Maps a normalized audio level (0.0 to 1.0) to a Unicode block meter glyph.
    pub fn unicode_glyph(normalized_level: f32) -> char {
        let level = normalized_level.clamp(0.0, 1.0);
        if level <= 0.05 {
            ' '
        } else if level < 0.18 {
            '▂'
        } else if level < 0.32 {
            '▃'
        } else if level < 0.46 {
            '▄'
        } else if level < 0.60 {
            '▅'
        } else if level < 0.74 {
            '▆'
        } else if level < 0.88 {
            '▇'
        } else {
            '█'
        }
    }

    /// Renders a horizontal Unicode VU meter of the specified column width.
    pub fn render_meter(rms_db: f32, width: usize) -> String {
        if width == 0 {
            return String::new();
        }
        // Normalize dBFS (-60 dBFS to 0 dBFS -> 0.0 to 1.0)
        let normalized = ((rms_db + 60.0) / 60.0).clamp(0.0, 1.0);
        let filled_chars = (normalized * width as f32).round() as usize;

        let mut out = String::with_capacity(width * 4);
        for i in 0..width {
            if i < filled_chars {
                let frac = (i + 1) as f32 / width as f32;
                if frac < 0.6 {
                    out.push('▰');
                } else if frac < 0.85 {
                    out.push('▰');
                } else {
                    out.push('▰');
                }
            } else {
                out.push('▱');
            }
        }
        out
    }

    /// Renders a dynamic sparkline from historical RMS level samples.
    pub fn render_sparkline(history: &[f32], max_width: usize) -> String {
        if history.is_empty() || max_width == 0 {
            return String::new();
        }
        let take_count = history.len().min(max_width);
        let slice = &history[history.len() - take_count..];

        slice
            .iter()
            .map(|&level| Self::unicode_glyph(level))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Speech-to-Text Providers & Configuration
// ---------------------------------------------------------------------------

/// Supported speech-to-text service providers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SttProvider {
    /// OpenAI Whisper API (`/v1/audio/transcriptions`).
    OpenAi,
    /// Groq Cloud fast Whisper API.
    Groq,
    /// Generic custom HTTP speech-to-text REST endpoint.
    CustomHttp,
    /// Locally installed Whisper CLI executable (`whisper` or `whisper-cpp`).
    LocalWhisper,
    /// In-memory mock STT adapter for testing and offline environments.
    Mock,
}

impl SttProvider {
    /// Canonical provider identifier string.
    pub fn name(&self) -> &'static str {
        match self {
            SttProvider::OpenAi => "openai",
            SttProvider::Groq => "groq",
            SttProvider::CustomHttp => "custom_http",
            SttProvider::LocalWhisper => "local_whisper",
            SttProvider::Mock => "mock",
        }
    }

    /// Default transcription model associated with this provider.
    pub fn default_model(&self) -> &'static str {
        match self {
            SttProvider::OpenAi => "whisper-1",
            SttProvider::Groq => "whisper-large-v3-turbo",
            SttProvider::CustomHttp => "default",
            SttProvider::LocalWhisper => "base.en",
            SttProvider::Mock => "mock-whisper",
        }
    }

    /// Default HTTP endpoint URL associated with this provider (if cloud-based).
    pub fn default_endpoint(&self) -> Option<&'static str> {
        match self {
            SttProvider::OpenAi => Some("https://api.openai.com/v1/audio/transcriptions"),
            SttProvider::Groq => Some("https://api.groq.com/openai/v1/audio/transcriptions"),
            SttProvider::CustomHttp => None,
            SttProvider::LocalWhisper => None,
            SttProvider::Mock => None,
        }
    }

    /// Parses an STT provider identifier from string.
    pub fn from_str_name(name: &str) -> Option<Self> {
        let cleaned = name.trim().to_lowercase();
        match cleaned.as_str() {
            "openai" | "whisper" | "openai-whisper" => Some(SttProvider::OpenAi),
            "groq" | "groq-whisper" => Some(SttProvider::Groq),
            "custom" | "http" | "custom_http" | "rest" => Some(SttProvider::CustomHttp),
            "local" | "whisper-cli" | "whisper_cpp" | "local_whisper" => {
                Some(SttProvider::LocalWhisper)
            }
            "mock" | "test" | "dummy" => Some(SttProvider::Mock),
            _ => None,
        }
    }
}

impl Default for SttProvider {
    fn default() -> Self {
        SttProvider::OpenAi
    }
}

impl fmt::Display for SttProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}
// ---------------------------------------------------------------------------
// Text-to-Speech Platform & Output Configuration
// ---------------------------------------------------------------------------

/// Supported platform backends for native text-to-speech output.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsPlatform {
    /// macOS native `say` utility.
    #[serde(rename = "macos", alias = "say")]
    MacOS,
    /// Linux Speech Dispatcher `spd-say` utility.
    #[serde(rename = "linux", alias = "spd_say", alias = "spd-say")]
    Linux,
    /// Windows PowerShell with System.Speech synthesis.
    #[serde(rename = "windows", alias = "powershell")]
    Windows,
    /// Custom CLI executable or template string (e.g. "espeak {text}").
    #[serde(rename = "custom")]
    Custom(String),
    /// In-memory mock/simulated synthesizer for tests and headless environments.
    #[serde(rename = "mock")]
    Mock,
    /// Automatically detected platform for host operating system.
    #[serde(rename = "auto")]
    Auto,
}

impl TtsPlatform {
    /// Returns the active platform backend corresponding to the host operating system.
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::MacOS
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Self::Mock
        }
    }

    /// Resolves `Auto` to the concrete host platform.
    pub fn resolve(&self) -> Self {
        match self {
            Self::Auto => Self::current(),
            other => other.clone(),
        }
    }

    /// Returns the primary binary or executable name for this platform.
    pub fn binary_name(&self) -> &'static str {
        match self.resolve() {
            Self::MacOS => "say",
            Self::Linux => "spd-say",
            Self::Windows => "powershell",
            Self::Custom(_) => "custom",
            Self::Mock => "mock",
            Self::Auto => "auto",
        }
    }

    /// Parses a string representation into a `TtsPlatform`.
    pub fn from_str_name(name: &str) -> Option<Self> {
        let cleaned = name.trim().to_lowercase();
        match cleaned.as_str() {
            "macos" | "say" | "darwin" | "apple" | "mac" => Some(Self::MacOS),
            "linux" | "spd-say" | "spd_say" | "speech-dispatcher" => Some(Self::Linux),
            "windows" | "powershell" | "sapi" | "win" | "win32" => Some(Self::Windows),
            "mock" | "test" | "dummy" | "simulated" => Some(Self::Mock),
            "auto" | "default" | "native" => Some(Self::Auto),
            s if s.starts_with("custom:") => Some(Self::Custom(s[7..].trim().to_string())),
            _ => None,
        }
    }
}

impl Default for TtsPlatform {
    fn default() -> Self {
        Self::Auto
    }
}

impl fmt::Display for TtsPlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MacOS => write!(f, "macos (say)"),
            Self::Linux => write!(f, "linux (spd-say)"),
            Self::Windows => write!(f, "windows (powershell)"),
            Self::Custom(cmd) => write!(f, "custom ({})", cmd),
            Self::Mock => write!(f, "mock"),
            Self::Auto => write!(f, "auto"),
        }
    }
}

/// Standalone configuration for Text-to-Speech (TTS) announcements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TtsConfig {
    /// Whether text-to-speech announcements are enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Whether voice output is globally muted.
    #[serde(default)]
    pub muted: bool,

    /// Speech synthesis platform backend.
    #[serde(default = "default_tts_platform")]
    pub platform: TtsPlatform,

    /// Speech rate multiplier (1.0 = normal, 0.5 = slow, 2.0 = fast).
    #[serde(default = "default_speech_rate")]
    pub speech_rate: f32,

    /// Voice identifier or name (e.g. "Samantha", "Alex", "Victoria", "David").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,

    /// Pitch adjustment modifier (-100 to 100 for Linux/spd-say).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<i32>,

    /// Output volume (0.0 to 1.0, default 1.0).
    #[serde(default = "default_volume")]
    pub volume: f32,

    /// Custom command template when platform is `TtsPlatform::Custom`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_command: Option<String>,

    /// Execution timeout in milliseconds (default 10,000 ms).
    #[serde(default = "default_tts_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_speech_rate() -> f32 {
    1.0
}

fn default_volume() -> f32 {
    1.0
}

fn default_tts_timeout_ms() -> u64 {
    10_000
}

fn default_tts_platform() -> TtsPlatform {
    TtsPlatform::Auto
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl TtsConfig {
    /// Creates a new `TtsConfig` with default settings.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            muted: false,
            platform: TtsPlatform::Auto,
            speech_rate: 1.0,
            voice: None,
            pitch: None,
            volume: 1.0,
            custom_command: None,
            timeout_ms: 10_000,
        }
    }

    /// Loads TTS configuration from environment variables:
    /// - `FUSION_TTS_ENABLED` / `TTS_ENABLED`: "1", "true", "yes"
    /// - `FUSION_VOICE_MUTED` / `VOICE_MUTED` / `FUSION_MUTED`: "1", "true", "yes"
    /// - `FUSION_TTS_PLATFORM` / `TTS_PLATFORM`: "macos", "linux", "windows", "mock"
    /// - `FUSION_TTS_RATE` / `TTS_RATE`: Floating point speed (e.g. "1.2")
    /// - `FUSION_TTS_VOICE` / `TTS_VOICE`: Voice name string
    /// - `FUSION_TTS_VOLUME` / `TTS_VOLUME`: Volume (0.0 to 1.0)
    /// - `FUSION_TTS_COMMAND` / `TTS_COMMAND`: Custom template
    pub fn from_env() -> Self {
        let mut config = Self::new(false);

        // Enable flags
        for var in &["FUSION_TTS_ENABLED", "TTS_ENABLED", "FUSION_TTS"] {
            if let Ok(val) = std::env::var(var) {
                let trimmed = val.trim().to_lowercase();
                if trimmed == "1" || trimmed == "true" || trimmed == "yes" || trimmed == "on" {
                    config.enabled = true;
                    break;
                } else if trimmed == "0"
                    || trimmed == "false"
                    || trimmed == "no"
                    || trimmed == "off"
                {
                    config.enabled = false;
                    break;
                }
            }
        }

        // Mute flags
        for var in &["FUSION_VOICE_MUTED", "VOICE_MUTED", "FUSION_MUTED", "MUTED"] {
            if let Ok(val) = std::env::var(var) {
                let trimmed = val.trim().to_lowercase();
                if trimmed == "1" || trimmed == "true" || trimmed == "yes" || trimmed == "on" {
                    config.muted = true;
                    break;
                } else if trimmed == "0"
                    || trimmed == "false"
                    || trimmed == "no"
                    || trimmed == "off"
                {
                    config.muted = false;
                    break;
                }
            }
        }

        // Platform
        if let Ok(p_str) =
            std::env::var("FUSION_TTS_PLATFORM").or_else(|_| std::env::var("TTS_PLATFORM"))
        {
            if let Some(p) = TtsPlatform::from_str_name(&p_str) {
                config.platform = p;
            }
        }

        // Speech Rate
        if let Ok(r_str) = std::env::var("FUSION_TTS_RATE").or_else(|_| std::env::var("TTS_RATE")) {
            if let Ok(r) = r_str.trim().parse::<f32>() {
                if r > 0.05 && r < 10.0 {
                    config.speech_rate = r;
                }
            }
        }

        // Voice
        if let Ok(v) = std::env::var("FUSION_TTS_VOICE").or_else(|_| std::env::var("TTS_VOICE")) {
            let v_clean = v.trim().to_string();
            if !v_clean.is_empty() {
                config.voice = Some(v_clean);
            }
        }

        // Volume
        if let Ok(vol_str) =
            std::env::var("FUSION_TTS_VOLUME").or_else(|_| std::env::var("TTS_VOLUME"))
        {
            if let Ok(vol) = vol_str.trim().parse::<f32>() {
                config.volume = vol.clamp(0.0, 1.0);
            }
        }

        // Custom command
        if let Ok(cmd) =
            std::env::var("FUSION_TTS_COMMAND").or_else(|_| std::env::var("TTS_COMMAND"))
        {
            let cmd_clean = cmd.trim().to_string();
            if !cmd_clean.is_empty() {
                config.custom_command = Some(cmd_clean);
            }
        }

        config
    }

    /// Builder method to enable/disable TTS.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Builder method to configure mute state.
    pub fn with_muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    /// Builder method to configure TTS platform backend.
    pub fn with_platform(mut self, platform: TtsPlatform) -> Self {
        self.platform = platform;
        self
    }

    /// Builder method to configure speech rate multiplier.
    pub fn with_speech_rate(mut self, rate: f32) -> Self {
        self.speech_rate = rate.clamp(0.1, 5.0);
        self
    }

    /// Builder method to configure voice identifier.
    pub fn with_voice(mut self, voice: impl Into<String>) -> Self {
        self.voice = Some(voice.into());
        self
    }

    /// Builder method to configure pitch.
    pub fn with_pitch(mut self, pitch: i32) -> Self {
        self.pitch = Some(pitch);
        self
    }

    /// Builder method to configure volume (0.0 to 1.0).
    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = volume.clamp(0.0, 1.0);
        self
    }

    /// Builder method to configure custom command template.
    pub fn with_custom_command(mut self, cmd: impl Into<String>) -> Self {
        self.custom_command = Some(cmd.into());
        self
    }

    /// Returns `true` if TTS output is enabled and not muted.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.muted
    }

    /// Builds the platform-specific CLI command and arguments for speaking the given text.
    ///
    /// Returns `(program, arguments)` suitable for `std::process::Command`.
    /// Returns `Err` if the platform is unsupported or if TTS is muted/disabled.
    pub fn build_command(&self, text: &str) -> Result<(String, Vec<String>), VoiceError> {
        if !self.is_active() {
            return Err(VoiceError::Muted);
        }

        let platform = self.platform.resolve();
        match platform {
            TtsPlatform::MacOS => {
                let mut args = Vec::new();
                // Speech rate: macOS `say` uses words-per-minute (default ~175-200).
                // We treat 1.0 as 200 WPM.
                let wpm = (self.speech_rate * 200.0).round() as i32;
                args.push("-r".to_string());
                args.push(wpm.to_string());
                if let Some(ref voice) = self.voice {
                    args.push("-v".to_string());
                    args.push(voice.clone());
                }
                args.push("--".to_string());
                args.push(text.to_string());
                Ok(("say".to_string(), args))
            }
            TtsPlatform::Linux => {
                let mut args = Vec::new();
                // spd-say uses -r for rate (-100 to 100, 0 = normal).
                // Map 1.0 -> 0, 0.5 -> -50, 2.0 -> 100.
                let rate = ((self.speech_rate - 1.0) * 100.0)
                    .clamp(-100.0, 100.0)
                    .round() as i32;
                args.push("-r".to_string());
                args.push(rate.to_string());
                if let Some(ref voice) = self.voice {
                    args.push("-t".to_string());
                    args.push(voice.clone());
                }
                if let Some(pitch) = self.pitch {
                    args.push("-p".to_string());
                    args.push(pitch.to_string());
                }
                // Volume: spd-say uses -i for volume (-100 to 100, 0 = normal).
                let vol = ((self.volume * 200.0) - 100.0).clamp(-100.0, 100.0).round() as i32;
                args.push("-i".to_string());
                args.push(vol.to_string());
                // Wait for speech to finish before returning.
                args.push("-w".to_string());
                args.push("--".to_string());
                args.push(text.to_string());
                Ok(("spd-say".to_string(), args))
            }
            TtsPlatform::Windows => {
                // Use PowerShell with System.Speech.Synthesis.SpeechSynthesizer.
                let mut script = String::new();
                script.push_str("Add-Type -AssemblyName System.Speech;");
                script.push_str("$s=New-Object System.Speech.Synthesis.SpeechSynthesizer;");
                if let Some(ref voice) = self.voice {
                    script.push_str(&format!("$s.SelectVoice('{}');", voice.replace('\'', "''")));
                }
                // Rate: SAPI uses -10 to 10, 0 = normal.
                let rate = ((self.speech_rate - 1.0) * 5.0).clamp(-10.0, 10.0).round() as i32;
                script.push_str(&format!("$s.Rate={};", rate));
                // Volume: SAPI uses 0 to 100.
                let vol = (self.volume * 100.0).clamp(0.0, 100.0).round() as i32;
                script.push_str(&format!("$s.Volume={};", vol));
                // Escape single quotes in text for PowerShell.
                let escaped = text.replace('\'', "''");
                script.push_str(&format!("$s.Speak('{}');", escaped));
                script.push_str("$s.Dispose()");
                Ok((
                    "powershell".to_string(),
                    vec![
                        "-NoProfile".to_string(),
                        "-NonInteractive".to_string(),
                        "-Command".to_string(),
                        script,
                    ],
                ))
            }
            TtsPlatform::Custom(ref cmd_template) => {
                let template = self
                    .custom_command
                    .as_deref()
                    .unwrap_or(cmd_template.as_str());
                if template.is_empty() {
                    return Err(VoiceError::TtsError(
                        "custom TTS command template is empty".to_string(),
                    ));
                }
                // Replace {text} placeholder, or append text as last argument.
                let expanded = if template.contains("{text}") {
                    template.replace("{text}", text)
                } else {
                    format!("{} {}", template, shell_escape(text))
                };
                // Split on whitespace for program + args (simple tokenization).
                let parts: Vec<&str> = expanded.split_whitespace().collect();
                if parts.is_empty() {
                    return Err(VoiceError::TtsError(
                        "custom TTS command resolved to empty".to_string(),
                    ));
                }
                Ok((
                    parts[0].to_string(),
                    parts[1..].iter().map(|s| s.to_string()).collect(),
                ))
            }
            TtsPlatform::Mock => {
                // Mock returns a no-op command for testing.
                Ok(("echo".to_string(), vec![text.to_string()]))
            }
            TtsPlatform::Auto => {
                // Should have been resolved above, but handle defensively.
                Err(VoiceError::UnsupportedPlatform(
                    "auto platform could not be resolved".to_string(),
                ))
            }
        }
    }
}

/// Escapes `text` for safe inclusion as a single shell argument using single-quote quoting.
fn shell_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for c in text.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Comprehensive configuration for audio capture, speech-to-text transcription,
/// text-to-speech output, and procedural audio chimes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// Whether voice input is globally enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Selected speech-to-text provider.
    #[serde(default)]
    pub provider: SttProvider,

    /// Model name passed to the transcription service.
    #[serde(default = "default_stt_model")]
    pub model: String,

    /// API authentication key (OpenAI key, Groq key, or custom bearer token).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Custom endpoint URL overriding the provider's default URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Optional target language code (e.g. "en", "es", "ja", "auto").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    /// Optional transcription prompt / context guide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    /// Optional temperature parameter (0.0 to 1.0) for model sampling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Audio sampling rate in Hz (default 16000).
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,

    /// Number of audio channels (default 1 = mono).
    #[serde(default = "default_channels")]
    pub channels: u16,

    /// Maximum recording duration in seconds before automatically stopping.
    #[serde(default = "default_max_duration")]
    pub max_duration_secs: u32,

    /// Silence threshold in dBFS for Voice Activity Detection.
    #[serde(default = "default_silence_threshold")]
    pub silence_threshold_db: f32,

    /// Duration of silence in milliseconds to trigger automatic completion.
    #[serde(default = "default_silence_timeout")]
    pub silence_timeout_ms: u64,

    /// Whether Voice Activity Detection (VAD) is active.
    #[serde(default = "default_true")]
    pub vad_enabled: bool,

    /// Automatically submit transcribed text directly to the REPL / agent prompt.
    #[serde(default = "default_true")]
    pub auto_submit: bool,

    /// Optional hardware input device name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,

    /// Whether text-to-speech announcement output is enabled.
    #[serde(default)]
    pub tts_enabled: bool,

    /// Whether voice and chime outputs are globally muted.
    #[serde(default)]
    pub tts_muted: bool,

    /// Speech rate multiplier for TTS output (default 1.0).
    #[serde(default = "default_speech_rate")]
    pub tts_rate: f32,

    /// Selected voice name for TTS output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_voice: Option<String>,

    /// Platform backend for TTS synthesis.
    #[serde(default = "default_tts_platform")]
    pub tts_platform: TtsPlatform,

    /// Whether audio alert chimes are enabled.
    #[serde(default = "default_true")]
    pub chimes_enabled: bool,
}

fn default_stt_model() -> String {
    "whisper-1".to_string()
}

fn default_sample_rate() -> u32 {
    DEFAULT_SAMPLE_RATE
}

fn default_channels() -> u16 {
    DEFAULT_CHANNELS
}

fn default_max_duration() -> u32 {
    DEFAULT_MAX_RECORDING_SECS
}

fn default_silence_threshold() -> f32 {
    DEFAULT_SILENCE_THRESHOLD_DB
}

fn default_silence_timeout() -> u64 {
    DEFAULT_SILENCE_TIMEOUT_MS
}

fn default_true() -> bool {
    true
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl VoiceConfig {
    /// Creates a new `VoiceConfig` with default settings.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            provider: SttProvider::OpenAi,
            model: "whisper-1".to_string(),
            api_key: None,
            endpoint: None,
            language: None,
            prompt: None,
            temperature: Some(0.0),
            sample_rate: DEFAULT_SAMPLE_RATE,
            channels: DEFAULT_CHANNELS,
            max_duration_secs: DEFAULT_MAX_RECORDING_SECS,
            silence_threshold_db: DEFAULT_SILENCE_THRESHOLD_DB,
            silence_timeout_ms: DEFAULT_SILENCE_TIMEOUT_MS,
            vad_enabled: true,
            auto_submit: true,
            device_name: None,
            tts_enabled: false,
            tts_muted: false,
            tts_rate: 1.0,
            tts_voice: None,
            tts_platform: TtsPlatform::Auto,
            chimes_enabled: true,
        }
    }

    /// Loads voice configuration from environment variables:
    /// - `FUSION_VOICE_ENABLED` / `VOICE_ENABLED`: "1", "true", "yes"
    /// - `FUSION_STT_PROVIDER`: "openai", "groq", "custom_http", "local_whisper", "mock"
    /// - `FUSION_STT_MODEL` / `WHISPER_MODEL`: Model identifier
    /// - `FUSION_STT_KEY` / `OPENAI_API_KEY` / `GROQ_API_KEY`: API authentication key
    /// - `FUSION_STT_ENDPOINT` / `WHISPER_ENDPOINT`: Custom HTTP endpoint URL
    /// - `FUSION_VOICE_LANG` / `VOICE_LANGUAGE`: Target language (e.g. "en")
    /// - `FUSION_VOICE_AUTO_SUBMIT`: Auto-submit to prompt
    /// - `FUSION_TTS_ENABLED` / `TTS_ENABLED`: Text-to-speech output flag
    /// - `FUSION_VOICE_MUTED` / `VOICE_MUTED`: Mute all audio/voice outputs
    /// - `FUSION_TTS_PLATFORM` / `TTS_PLATFORM`: "macos", "linux", "windows", "mock"
    /// - `FUSION_TTS_RATE` / `TTS_RATE`: Speech speed rate
    /// - `FUSION_TTS_VOICE` / `TTS_VOICE`: Voice identifier
    /// - `FUSION_CHIMES_ENABLED` / `CHIMES_ENABLED`: Alert chimes flag
    pub fn from_env() -> Self {
        let mut config = Self::new(false);

        // Check enable flags
        for var in &["FUSION_VOICE_ENABLED", "VOICE_ENABLED", "FUSION_VOICE"] {
            if let Ok(val) = std::env::var(var) {
                let trimmed = val.trim().to_lowercase();
                if trimmed == "1" || trimmed == "true" || trimmed == "yes" || trimmed == "on" {
                    config.enabled = true;
                    break;
                } else if trimmed == "0"
                    || trimmed == "false"
                    || trimmed == "no"
                    || trimmed == "off"
                {
                    config.enabled = false;
                    break;
                }
            }
        }

        // Provider
        if let Ok(p_str) =
            std::env::var("FUSION_STT_PROVIDER").or_else(|_| std::env::var("STT_PROVIDER"))
        {
            if let Some(p) = SttProvider::from_str_name(&p_str) {
                config.provider = p;
            }
        }

        // Model
        if let Ok(m) = std::env::var("FUSION_STT_MODEL").or_else(|_| std::env::var("WHISPER_MODEL"))
        {
            let m_clean = m.trim().to_string();
            if !m_clean.is_empty() {
                config.model = m_clean;
            }
        } else {
            config.model = config.provider.default_model().to_string();
        }

        // API Key
        let key = std::env::var("FUSION_STT_KEY")
            .or_else(|_| match config.provider {
                SttProvider::OpenAi => std::env::var("OPENAI_API_KEY"),
                SttProvider::Groq => std::env::var("GROQ_API_KEY"),
                _ => std::env::var("OPENAI_API_KEY"),
            })
            .ok()
            .and_then(|k| {
                let trimmed = k.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            });
        config.api_key = key;

        // Endpoint
        if let Ok(ep) =
            std::env::var("FUSION_STT_ENDPOINT").or_else(|_| std::env::var("WHISPER_ENDPOINT"))
        {
            let ep_clean = ep.trim().to_string();
            if !ep_clean.is_empty() {
                config.endpoint = Some(ep_clean);
            }
        }

        // Language
        if let Ok(lang) =
            std::env::var("FUSION_VOICE_LANG").or_else(|_| std::env::var("VOICE_LANGUAGE"))
        {
            let lang_clean = lang.trim().to_string();
            if !lang_clean.is_empty() {
                config.language = Some(lang_clean);
            }
        }

        // Auto submit
        if let Ok(auto) = std::env::var("FUSION_VOICE_AUTO_SUBMIT") {
            let trimmed = auto.trim().to_lowercase();
            config.auto_submit = trimmed != "0" && trimmed != "false" && trimmed != "no";
        }

        // TTS enable flags
        for var in &["FUSION_TTS_ENABLED", "TTS_ENABLED", "FUSION_TTS"] {
            if let Ok(val) = std::env::var(var) {
                let trimmed = val.trim().to_lowercase();
                if trimmed == "1" || trimmed == "true" || trimmed == "yes" || trimmed == "on" {
                    config.tts_enabled = true;
                    break;
                } else if trimmed == "0"
                    || trimmed == "false"
                    || trimmed == "no"
                    || trimmed == "off"
                {
                    config.tts_enabled = false;
                    break;
                }
            }
        }

        // Voice mute flags
        for var in &["FUSION_VOICE_MUTED", "VOICE_MUTED", "FUSION_MUTED", "MUTED"] {
            if let Ok(val) = std::env::var(var) {
                let trimmed = val.trim().to_lowercase();
                if trimmed == "1" || trimmed == "true" || trimmed == "yes" || trimmed == "on" {
                    config.tts_muted = true;
                    break;
                } else if trimmed == "0"
                    || trimmed == "false"
                    || trimmed == "no"
                    || trimmed == "off"
                {
                    config.tts_muted = false;
                    break;
                }
            }
        }

        // TTS Platform
        if let Ok(p_str) =
            std::env::var("FUSION_TTS_PLATFORM").or_else(|_| std::env::var("TTS_PLATFORM"))
        {
            if let Some(p) = TtsPlatform::from_str_name(&p_str) {
                config.tts_platform = p;
            }
        }

        // TTS Speech Rate
        if let Ok(r_str) = std::env::var("FUSION_TTS_RATE").or_else(|_| std::env::var("TTS_RATE")) {
            if let Ok(r) = r_str.trim().parse::<f32>() {
                if r > 0.05 && r < 10.0 {
                    config.tts_rate = r;
                }
            }
        }

        // TTS Voice
        if let Ok(v) = std::env::var("FUSION_TTS_VOICE").or_else(|_| std::env::var("TTS_VOICE")) {
            let v_clean = v.trim().to_string();
            if !v_clean.is_empty() {
                config.tts_voice = Some(v_clean);
            }
        }

        // Chimes enable flags
        for var in &["FUSION_CHIMES_ENABLED", "CHIMES_ENABLED", "FUSION_CHIMES"] {
            if let Ok(val) = std::env::var(var) {
                let trimmed = val.trim().to_lowercase();
                if trimmed == "0" || trimmed == "false" || trimmed == "no" || trimmed == "off" {
                    config.chimes_enabled = false;
                    break;
                }
            }
        }

        config
    }

    /// Builder method to configure provider.
    pub fn with_provider(mut self, provider: SttProvider) -> Self {
        self.provider = provider;
        self
    }

    /// Builder method to configure model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Builder method to configure API key.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Builder method to configure endpoint.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Builder method to configure target transcription language.
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Builder method to configure silence detection threshold.
    pub fn with_silence_threshold_db(mut self, threshold_db: f32) -> Self {
        self.silence_threshold_db = threshold_db;
        self
    }

    /// Builder method to configure silence timeout.
    pub fn with_silence_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.silence_timeout_ms = timeout_ms;
        self
    }

    /// Builder method to configure TTS enabled flag.
    pub fn with_tts(mut self, enabled: bool) -> Self {
        self.tts_enabled = enabled;
        self
    }

    /// Builder method to configure voice mute flag.
    pub fn with_tts_muted(mut self, muted: bool) -> Self {
        self.tts_muted = muted;
        self
    }

    /// Builder method to configure TTS speech rate multiplier.
    pub fn with_tts_rate(mut self, rate: f32) -> Self {
        self.tts_rate = rate.clamp(0.1, 5.0);
        self
    }

    /// Builder method to configure TTS voice identifier.
    pub fn with_tts_voice(mut self, voice: impl Into<String>) -> Self {
        self.tts_voice = Some(voice.into());
        self
    }

    /// Builder method to configure TTS platform.
    pub fn with_tts_platform(mut self, platform: TtsPlatform) -> Self {
        self.tts_platform = platform;
        self
    }

    /// Builder method to configure audio alert chimes enabled flag.
    pub fn with_chimes(mut self, chimes: bool) -> Self {
        self.chimes_enabled = chimes;
        self
    }

    /// Converts this `VoiceConfig` into a `TtsConfig`.
    pub fn to_tts_config(&self) -> TtsConfig {
        TtsConfig {
            enabled: self.tts_enabled,
            muted: self.tts_muted,
            platform: self.tts_platform.clone(),
            speech_rate: self.tts_rate,
            voice: self.tts_voice.clone(),
            pitch: None,
            volume: 1.0,
            custom_command: None,
            timeout_ms: 10_000,
        }
    }

    /// Returns the effective HTTP endpoint for this configuration.
    pub fn effective_endpoint(&self) -> Option<String> {
        self.endpoint
            .clone()
            .or_else(|| self.provider.default_endpoint().map(|s| s.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Transcription Request & Result Types
// ---------------------------------------------------------------------------

/// Request payload containing audio data and transcription parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionRequest {
    /// Raw audio file binary bytes (e.g. WAV, MP3, OGG).
    pub audio_bytes: Vec<u8>,
    /// Audio format of the payload.
    pub format: AudioFormat,
    /// Upload filename with proper extension.
    pub filename: String,
    /// Model name to use for transcription.
    pub model: String,
    /// Optional ISO-639-1 language code.
    pub language: Option<String>,
    /// Optional guiding prompt for context.
    pub prompt: Option<String>,
    /// Optional model sampling temperature.
    pub temperature: Option<f32>,
}

impl TranscriptionRequest {
    /// Creates a transcription request from an [`AudioBuffer`] by encoding to WAV.
    pub fn from_audio_buffer(buffer: &AudioBuffer, config: &VoiceConfig) -> Self {
        let wav_bytes = buffer.to_wav_bytes();
        Self {
            audio_bytes: wav_bytes,
            format: AudioFormat::Wav,
            filename: "fusion_voice_input.wav".to_string(),
            model: config.model.clone(),
            language: config.language.clone(),
            prompt: config.prompt.clone(),
            temperature: config.temperature,
        }
    }

    /// Creates a transcription request from raw audio bytes.
    pub fn from_bytes(bytes: Vec<u8>, format: AudioFormat, model: impl Into<String>) -> Self {
        let ext = format.file_extension();
        Self {
            audio_bytes: bytes,
            format,
            filename: format!("audio.{}", ext),
            model: model.into(),
            language: None,
            prompt: None,
            temperature: None,
        }
    }
}

/// Individual timestamped transcription segment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptionSegment {
    /// Segment index id.
    pub id: u32,
    /// Start timestamp in seconds.
    pub start: f64,
    /// End timestamp in seconds.
    pub end: f64,
    /// Transcribed text for this segment.
    pub text: String,
    /// Optional average log probability / confidence score.
    pub avg_logprob: Option<f64>,
}

/// Final transcribed text output from a speech-to-text service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptionResult {
    /// Complete concatenated transcription text.
    pub text: String,
    /// Detected or configured language code.
    pub language: Option<String>,
    /// Total duration of the transcribed audio in seconds.
    pub duration_secs: Option<f64>,
    /// Optional detailed timestamped segments.
    pub segments: Vec<TranscriptionSegment>,
    /// Identifier of the provider that produced this transcription.
    pub provider: String,
}

impl TranscriptionResult {
    /// Creates a simple transcription result with the provided text and provider name.
    pub fn new(text: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            language: None,
            duration_secs: None,
            segments: Vec::new(),
            provider: provider.into(),
        }
    }

    /// Appends a timestamped segment.
    pub fn with_segment(mut self, segment: TranscriptionSegment) -> Self {
        self.segments.push(segment);
        self
    }
}

// ---------------------------------------------------------------------------
// Speech-to-Text Transcription Adapter Trait
// ---------------------------------------------------------------------------

/// Universal asynchronous abstraction for speech-to-text transcription services.
#[async_trait]
pub trait SpeechToTextAdapter: Send + Sync {
    /// Transcribes the given audio request into text.
    async fn transcribe(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<TranscriptionResult, VoiceError>;

    /// Transcribes an in-memory [`AudioBuffer`].
    async fn transcribe_buffer(
        &self,
        buffer: &AudioBuffer,
        config: &VoiceConfig,
    ) -> Result<TranscriptionResult, VoiceError> {
        let request = TranscriptionRequest::from_audio_buffer(buffer, config);
        self.transcribe(&request).await
    }

    /// Checks if this adapter is configured and ready for requests.
    fn is_available(&self) -> bool;

    /// Returns the provider name.
    fn provider_name(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Pure Rust Multipart Form Data Builder (Zero Extra Dependencies)
// ---------------------------------------------------------------------------

/// Pure Rust multipart/form-data payload builder for HTTP audio upload.
struct MultipartBuilder {
    boundary: String,
    body: Vec<u8>,
}

impl MultipartBuilder {
    fn new() -> Self {
        let boundary = format!("----FusionBoundary{}", uuid::Uuid::new_v4().simple());
        Self {
            boundary,
            body: Vec::new(),
        }
    }

    fn add_text_field(&mut self, name: &str, value: &str) {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n",
                self.boundary, name, value
            )
            .as_bytes(),
        );
    }

    fn add_file_field(&mut self, name: &str, filename: &str, mime_type: &str, data: &[u8]) {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                self.boundary, name, filename, mime_type
            )
            .as_bytes(),
        );
        self.body.extend_from_slice(data);
        self.body.extend_from_slice(b"\r\n");
    }

    fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        let content_type = format!("multipart/form-data; boundary={}", self.boundary);
        (content_type, self.body)
    }
}

// ---------------------------------------------------------------------------
// OpenAI Whisper Adapter
// ---------------------------------------------------------------------------

/// Speech-to-text adapter for the OpenAI Whisper API.
pub struct OpenAiWhisperAdapter {
    api_key: String,
    endpoint: String,
    client: reqwest::Client,
}

impl OpenAiWhisperAdapter {
    /// Creates a new OpenAI Whisper adapter with the given API key and optional custom endpoint.
    pub fn new(api_key: impl Into<String>, custom_endpoint: Option<String>) -> Self {
        let endpoint = custom_endpoint
            .unwrap_or_else(|| "https://api.openai.com/v1/audio/transcriptions".to_string());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .unwrap_or_default();

        Self {
            api_key: api_key.into(),
            endpoint,
            client,
        }
    }

    /// Creates an adapter from `VoiceConfig`.
    pub fn from_config(config: &VoiceConfig) -> Result<Self, VoiceError> {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| {
                VoiceError::AuthError(
                    "Missing OpenAI API key for Whisper transcription".to_string(),
                )
            })?;

        let endpoint = config.effective_endpoint();
        Ok(Self::new(api_key, endpoint))
    }
}

#[async_trait]
impl SpeechToTextAdapter for OpenAiWhisperAdapter {
    async fn transcribe(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<TranscriptionResult, VoiceError> {
        if self.api_key.trim().is_empty() {
            return Err(VoiceError::AuthError("OpenAI API key is empty".to_string()));
        }

        let mut form = MultipartBuilder::new();
        form.add_text_field("model", &request.model);
        form.add_file_field(
            "file",
            &request.filename,
            request.format.mime_type(),
            &request.audio_bytes,
        );

        if let Some(lang) = &request.language {
            form.add_text_field("language", lang);
        }
        if let Some(prompt) = &request.prompt {
            form.add_text_field("prompt", prompt);
        }
        if let Some(temp) = request.temperature {
            form.add_text_field("temperature", &temp.to_string());
        }

        // Request verbose JSON to retrieve segments if available
        form.add_text_field("response_format", "verbose_json");

        let (content_type, body_bytes) = form.finish();

        let response = self
            .client
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", content_type)
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| VoiceError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(VoiceError::ApiError {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let json_body: serde_json::Value = response.json().await.map_err(|e| {
            VoiceError::EncodingError(format!("Failed to parse JSON response: {}", e))
        })?;

        let text = json_body["text"].as_str().unwrap_or("").trim().to_string();

        let language = json_body["language"].as_str().map(|s| s.to_string());
        let duration_secs = json_body["duration"].as_f64();

        let mut segments = Vec::new();
        if let Some(raw_segs) = json_body["segments"].as_array() {
            for (idx, s) in raw_segs.iter().enumerate() {
                let start = s["start"].as_f64().unwrap_or(0.0);
                let end = s["end"].as_f64().unwrap_or(0.0);
                let seg_text = s["text"].as_str().unwrap_or("").to_string();
                let avg_logprob = s["avg_logprob"].as_f64();

                segments.push(TranscriptionSegment {
                    id: idx as u32,
                    start,
                    end,
                    text: seg_text,
                    avg_logprob,
                });
            }
        }

        Ok(TranscriptionResult {
            text,
            language,
            duration_secs,
            segments,
            provider: "openai".to_string(),
        })
    }

    fn is_available(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    fn provider_name(&self) -> &'static str {
        "openai"
    }
}

// ---------------------------------------------------------------------------
// Groq Whisper Adapter
// ---------------------------------------------------------------------------

/// Speech-to-text adapter for Groq Cloud's low-latency Whisper API.
pub struct GroqWhisperAdapter {
    api_key: String,
    endpoint: String,
    client: reqwest::Client,
}

impl GroqWhisperAdapter {
    /// Creates a new Groq Whisper adapter.
    pub fn new(api_key: impl Into<String>, custom_endpoint: Option<String>) -> Self {
        let endpoint = custom_endpoint
            .unwrap_or_else(|| "https://api.groq.com/openai/v1/audio/transcriptions".to_string());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            api_key: api_key.into(),
            endpoint,
            client,
        }
    }

    /// Creates an adapter from `VoiceConfig`.
    pub fn from_config(config: &VoiceConfig) -> Result<Self, VoiceError> {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("GROQ_API_KEY").ok())
            .ok_or_else(|| {
                VoiceError::AuthError("Missing Groq API key for Whisper transcription".to_string())
            })?;

        let endpoint = config.effective_endpoint();
        Ok(Self::new(api_key, endpoint))
    }
}

#[async_trait]
impl SpeechToTextAdapter for GroqWhisperAdapter {
    async fn transcribe(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<TranscriptionResult, VoiceError> {
        if self.api_key.trim().is_empty() {
            return Err(VoiceError::AuthError("Groq API key is empty".to_string()));
        }

        let mut form = MultipartBuilder::new();
        // Default to whisper-large-v3-turbo for fast latency on Groq
        let model = if request.model.is_empty() || request.model == "whisper-1" {
            "whisper-large-v3-turbo"
        } else {
            &request.model
        };
        form.add_text_field("model", model);
        form.add_file_field(
            "file",
            &request.filename,
            request.format.mime_type(),
            &request.audio_bytes,
        );

        if let Some(lang) = &request.language {
            form.add_text_field("language", lang);
        }
        if let Some(prompt) = &request.prompt {
            form.add_text_field("prompt", prompt);
        }
        if let Some(temp) = request.temperature {
            form.add_text_field("temperature", &temp.to_string());
        }

        form.add_text_field("response_format", "verbose_json");

        let (content_type, body_bytes) = form.finish();

        let response = self
            .client
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", content_type)
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| VoiceError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(VoiceError::ApiError {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let json_body: serde_json::Value = response.json().await.map_err(|e| {
            VoiceError::EncodingError(format!("Failed to parse JSON response: {}", e))
        })?;

        let text = json_body["text"].as_str().unwrap_or("").trim().to_string();

        let language = json_body["language"].as_str().map(|s| s.to_string());
        let duration_secs = json_body["duration"].as_f64();

        Ok(TranscriptionResult {
            text,
            language,
            duration_secs,
            segments: Vec::new(),
            provider: "groq".to_string(),
        })
    }

    fn is_available(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    fn provider_name(&self) -> &'static str {
        "groq"
    }
}

// ---------------------------------------------------------------------------
// Custom HTTP STT Adapter
// ---------------------------------------------------------------------------

/// Generic speech-to-text adapter for custom REST HTTP endpoints.
pub struct CustomHttpSttAdapter {
    endpoint: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl CustomHttpSttAdapter {
    /// Creates a new custom HTTP STT adapter.
    pub fn new(endpoint: impl Into<String>, api_key: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Self {
            endpoint: endpoint.into(),
            api_key,
            client,
        }
    }

    /// Creates an adapter from `VoiceConfig`.
    pub fn from_config(config: &VoiceConfig) -> Result<Self, VoiceError> {
        let endpoint = config.endpoint.clone().ok_or_else(|| {
            VoiceError::NotConfigured("Missing endpoint URL for custom HTTP STT".to_string())
        })?;
        Ok(Self::new(endpoint, config.api_key.clone()))
    }
}

#[async_trait]
impl SpeechToTextAdapter for CustomHttpSttAdapter {
    async fn transcribe(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<TranscriptionResult, VoiceError> {
        let mut form = MultipartBuilder::new();
        form.add_text_field("model", &request.model);
        form.add_file_field(
            "file",
            &request.filename,
            request.format.mime_type(),
            &request.audio_bytes,
        );

        if let Some(lang) = &request.language {
            form.add_text_field("language", lang);
        }

        let (content_type, body_bytes) = form.finish();

        let mut req = self
            .client
            .post(&self.endpoint)
            .header("Content-Type", content_type);
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let response = req
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| VoiceError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let err = response.text().await.unwrap_or_default();
            return Err(VoiceError::ApiError {
                status: status.as_u16(),
                message: err,
            });
        }

        let json_body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| VoiceError::EncodingError(format!("Failed to parse response: {}", e)))?;

        // Extract transcription text from common schema fields ("text", "transcription", "transcript")
        let text = json_body["text"]
            .as_str()
            .or_else(|| json_body["transcription"].as_str())
            .or_else(|| json_body["transcript"].as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(TranscriptionResult {
            text,
            language: request.language.clone(),
            duration_secs: None,
            segments: Vec::new(),
            provider: "custom_http".to_string(),
        })
    }

    fn is_available(&self) -> bool {
        !self.endpoint.trim().is_empty()
    }

    fn provider_name(&self) -> &'static str {
        "custom_http"
    }
}

// ---------------------------------------------------------------------------
// Local CLI Whisper Adapter
// ---------------------------------------------------------------------------

/// Speech-to-text adapter that executes a local CLI binary (e.g. `whisper` or `whisper-cpp`).
pub struct LocalWhisperAdapter {
    executable: String,
    model: String,
}

impl LocalWhisperAdapter {
    /// Creates a new local CLI Whisper adapter.
    pub fn new(executable: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
            model: model.into(),
        }
    }

    /// Creates an adapter from `VoiceConfig`.
    pub fn from_config(config: &VoiceConfig) -> Self {
        let exe = std::env::var("WHISPER_PATH").unwrap_or_else(|_| "whisper".to_string());
        Self::new(exe, config.model.clone())
    }
}

#[async_trait]
impl SpeechToTextAdapter for LocalWhisperAdapter {
    async fn transcribe(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<TranscriptionResult, VoiceError> {
        let temp_dir = std::env::temp_dir().join(format!(
            "fusion-voice-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&temp_dir)
            .map_err(|e| VoiceError::ProcessError(format!("Failed to create temp dir: {}", e)))?;
        let temp_audio_path = temp_dir.join(&request.filename);

        tokio::fs::write(&temp_audio_path, &request.audio_bytes)
            .await
            .map_err(|e| {
                VoiceError::ProcessError(format!("Failed to write temp audio file: {}", e))
            })?;

        let mut cmd = tokio::process::Command::new(&self.executable);
        cmd.arg(&temp_audio_path)
            .arg("--model")
            .arg(&self.model)
            .arg("--output_format")
            .arg("txt")
            .arg("--output_dir")
            .arg(&temp_dir);

        if let Some(lang) = &request.language {
            cmd.arg("--language").arg(lang);
        }

        let output = cmd.output().await.map_err(|e| {
            VoiceError::ProcessError(format!("Failed to spawn local whisper CLI: {}", e))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VoiceError::ProcessError(format!(
                "Whisper CLI exited with error code: {}",
                stderr
            )));
        }

        // Read generated .txt file
        let txt_filename = temp_audio_path
            .file_stem()
            .map(|s| format!("{}.txt", s.to_string_lossy()))
            .unwrap_or_else(|| "output.txt".to_string());
        let txt_path = temp_dir.join(txt_filename);

        let transcribed_text = if txt_path.exists() {
            tokio::fs::read_to_string(&txt_path)
                .await
                .unwrap_or_default()
                .trim()
                .to_string()
        } else {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };

        Ok(TranscriptionResult {
            text: transcribed_text,
            language: request.language.clone(),
            duration_secs: None,
            segments: Vec::new(),
            provider: "local_whisper".to_string(),
        })
    }

    fn is_available(&self) -> bool {
        !self.executable.is_empty()
    }

    fn provider_name(&self) -> &'static str {
        "local_whisper"
    }
}

// ---------------------------------------------------------------------------
// In-Memory Mock STT Adapter (Testing & Simulation)
// ---------------------------------------------------------------------------

/// Deterministic mock speech-to-text adapter for automated testing and offline development.
#[derive(Debug, Clone)]
pub struct MockSttAdapter {
    canned_transcript: String,
    simulated_delay_ms: u64,
    fail_with_error: Option<VoiceError>,
}

impl MockSttAdapter {
    /// Creates a mock adapter returning the specified transcript.
    pub fn new(canned_transcript: impl Into<String>) -> Self {
        Self {
            canned_transcript: canned_transcript.into(),
            simulated_delay_ms: 0,
            fail_with_error: None,
        }
    }

    /// Sets simulated network latency.
    pub fn with_delay(mut self, delay_ms: u64) -> Self {
        self.simulated_delay_ms = delay_ms;
        self
    }

    /// Configures the mock to fail with a designated error.
    pub fn with_error(mut self, error: VoiceError) -> Self {
        self.fail_with_error = Some(error);
        self
    }
}

impl Default for MockSttAdapter {
    fn default() -> Self {
        Self::new("Hello Fusion! Show me the status of the repository.")
    }
}

#[async_trait]
impl SpeechToTextAdapter for MockSttAdapter {
    async fn transcribe(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<TranscriptionResult, VoiceError> {
        if self.simulated_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.simulated_delay_ms)).await;
        }

        if let Some(err) = &self.fail_with_error {
            return Err(err.clone());
        }

        let text = if self.canned_transcript.is_empty() {
            format!("Transcribed {} bytes of audio", request.audio_bytes.len())
        } else {
            self.canned_transcript.clone()
        };

        Ok(TranscriptionResult {
            text,
            language: request.language.clone().or_else(|| Some("en".to_string())),
            duration_secs: Some(2.5),
            segments: vec![TranscriptionSegment {
                id: 0,
                start: 0.0,
                end: 2.5,
                text: self.canned_transcript.clone(),
                avg_logprob: Some(-0.15),
            }],
            provider: "mock".to_string(),
        })
    }

    fn is_available(&self) -> bool {
        true
    }

    fn provider_name(&self) -> &'static str {
        "mock"
    }
}

// ---------------------------------------------------------------------------
// Factory Function
// ---------------------------------------------------------------------------

/// Creates a speech-to-text adapter instance corresponding to the active `VoiceConfig`.
pub fn create_stt_adapter(
    config: &VoiceConfig,
) -> Result<Box<dyn SpeechToTextAdapter>, VoiceError> {
    match config.provider {
        SttProvider::OpenAi => {
            let adapter = OpenAiWhisperAdapter::from_config(config)?;
            Ok(Box::new(adapter))
        }
        SttProvider::Groq => {
            let adapter = GroqWhisperAdapter::from_config(config)?;
            Ok(Box::new(adapter))
        }
        SttProvider::CustomHttp => {
            let adapter = CustomHttpSttAdapter::from_config(config)?;
            Ok(Box::new(adapter))
        }
        SttProvider::LocalWhisper => {
            let adapter = LocalWhisperAdapter::from_config(config);
            Ok(Box::new(adapter))
        }
        SttProvider::Mock => {
            let adapter = MockSttAdapter::default();
            Ok(Box::new(adapter))
        }
    }
}

// ---------------------------------------------------------------------------
// Voice Input State Machine & Recording Session
// ---------------------------------------------------------------------------

/// Current status of an interactive voice input session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VoiceInputState {
    /// Inactive / ready to begin recording.
    Idle,
    /// Listening for user to start speaking.
    Listening {
        duration_ms: u64,
        current_rms_db: f32,
    },
    /// Recording active voice speech input.
    Recording {
        duration_ms: u64,
        sample_count: usize,
        current_rms_db: f32,
    },
    /// Speech recording finalized; currently querying speech-to-text provider.
    Transcribing {
        duration_ms: u64,
        audio_duration_secs: f64,
    },
    /// Speech-to-text transcription successfully completed.
    Completed {
        text: String,
        duration_ms: u64,
        audio_duration_secs: f64,
    },
    /// Voice capture or transcription encountered an error.
    Failed { error: String },
    /// Cancelled by the user.
    Cancelled,
}

impl VoiceInputState {
    /// Returns a short display label for terminal UI prompts.
    pub fn label(&self) -> &'static str {
        match self {
            VoiceInputState::Idle => "Idle",
            VoiceInputState::Listening { .. } => "Listening...",
            VoiceInputState::Recording { .. } => "Recording",
            VoiceInputState::Transcribing { .. } => "Transcribing...",
            VoiceInputState::Completed { .. } => "Complete",
            VoiceInputState::Failed { .. } => "Error",
            VoiceInputState::Cancelled => "Cancelled",
        }
    }

    /// Whether this state represents an active recording or transcribing operation.
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            VoiceInputState::Listening { .. }
                | VoiceInputState::Recording { .. }
                | VoiceInputState::Transcribing { .. }
        )
    }

    /// Whether this state represents a terminal result (Completed, Failed, Cancelled).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            VoiceInputState::Completed { .. }
                | VoiceInputState::Failed { .. }
                | VoiceInputState::Cancelled
        )
    }
}

impl Default for VoiceInputState {
    fn default() -> Self {
        VoiceInputState::Idle
    }
}

/// Coordinates in-memory audio recording, Voice Activity Detection, and transcription.
pub struct VoiceSession {
    config: VoiceConfig,
    buffer: AudioBuffer,
    vad: VadDetector,
    state: VoiceInputState,
    start_time: Option<Instant>,
    rms_history: Vec<f32>,
    adapter: Box<dyn SpeechToTextAdapter>,
    cancelled: Arc<AtomicBool>,
}

impl VoiceSession {
    /// Creates a new voice session with the given configuration and STT adapter.
    pub fn new(config: VoiceConfig, adapter: Box<dyn SpeechToTextAdapter>) -> Self {
        let vad_cfg = VadConfig {
            threshold_db: config.silence_threshold_db,
            silence_timeout_ms: config.silence_timeout_ms,
            min_speech_duration_ms: 200,
        };
        let sample_rate = config.sample_rate;
        let channels = config.channels;

        Self {
            config,
            buffer: AudioBuffer::new(sample_rate, channels),
            vad: VadDetector::new(vad_cfg),
            state: VoiceInputState::Idle,
            start_time: None,
            rms_history: Vec::with_capacity(64),
            adapter,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Starts a recording session.
    pub fn start(&mut self) {
        self.buffer.clear();
        self.vad.reset();
        self.rms_history.clear();
        self.start_time = Some(Instant::now());
        self.cancelled.store(false, Ordering::SeqCst);
        self.state = VoiceInputState::Listening {
            duration_ms: 0,
            current_rms_db: -96.0,
        };
    }

    /// Feeds incoming PCM samples into the session, updating VAD and session state.
    ///
    /// Returns `true` if Voice Activity Detection detected that speech has ended
    /// and the recording is ready to be transcribed.
    pub fn feed_samples(&mut self, samples: &[i16]) -> bool {
        if !self.state.is_active() {
            return false;
        }

        if self.cancelled.load(Ordering::Relaxed) {
            self.state = VoiceInputState::Cancelled;
            return false;
        }

        let chunk = AudioBuffer::from_i16_samples(
            samples.to_vec(),
            self.buffer.sample_rate,
            self.buffer.channels,
        );

        let rms_val = chunk.rms();
        let rms_db = chunk.rms_db();

        if self.rms_history.len() >= 40 {
            self.rms_history.remove(0);
        }
        self.rms_history.push(rms_val);

        self.buffer.push_samples(samples);

        let elapsed_ms = self
            .start_time
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);

        // Check maximum duration limit
        let max_ms = (self.config.max_duration_secs as u64) * 1000;
        if elapsed_ms >= max_ms {
            return true;
        }

        let vad_state = self.vad.process_chunk(&chunk);

        match vad_state {
            VadState::Listening => {
                self.state = VoiceInputState::Listening {
                    duration_ms: elapsed_ms,
                    current_rms_db: rms_db,
                };
                false
            }
            VadState::Speaking => {
                self.state = VoiceInputState::Recording {
                    duration_ms: elapsed_ms,
                    sample_count: self.buffer.len(),
                    current_rms_db: rms_db,
                };
                false
            }
            VadState::SpeechEnded => {
                self.state = VoiceInputState::Recording {
                    duration_ms: elapsed_ms,
                    sample_count: self.buffer.len(),
                    current_rms_db: rms_db,
                };
                true
            }
        }
    }

    /// Finalizes recording and executes speech-to-text transcription via the configured adapter.
    pub async fn finish_and_transcribe(&mut self) -> Result<TranscriptionResult, VoiceError> {
        if self.cancelled.load(Ordering::Relaxed) {
            self.state = VoiceInputState::Cancelled;
            return Err(VoiceError::Cancelled);
        }

        let total_duration_secs = self.buffer.duration_secs();
        let elapsed_ms = self
            .start_time
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);

        if self.buffer.is_empty() || total_duration_secs < 0.1 {
            self.state = VoiceInputState::Failed {
                error: "Audio recording is empty".to_string(),
            };
            return Err(VoiceError::NoSpeechDetected);
        }

        self.state = VoiceInputState::Transcribing {
            duration_ms: elapsed_ms,
            audio_duration_secs: total_duration_secs,
        };

        let request = TranscriptionRequest::from_audio_buffer(&self.buffer, &self.config);

        match self.adapter.transcribe(&request).await {
            Ok(result) => {
                self.state = VoiceInputState::Completed {
                    text: result.text.clone(),
                    duration_ms: elapsed_ms,
                    audio_duration_secs: total_duration_secs,
                };
                Ok(result)
            }
            Err(err) => {
                self.state = VoiceInputState::Failed {
                    error: err.to_string(),
                };
                Err(err)
            }
        }
    }

    /// Cancels the current recording session.
    pub fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.state = VoiceInputState::Cancelled;
    }

    /// Returns the current state of the voice input session.
    pub fn state(&self) -> &VoiceInputState {
        &self.state
    }

    /// Returns a reference to the internal accumulated audio buffer.
    pub fn buffer(&self) -> &AudioBuffer {
        &self.buffer
    }

    /// Returns the recent RMS amplitude history.
    pub fn rms_history(&self) -> &[f32] {
        &self.rms_history
    }
}

// ---------------------------------------------------------------------------
// Terminal UI Badges & Formatted Visualizations
// ---------------------------------------------------------------------------

/// Renders a compact ANSI-styled status badge representing the voice input state.
pub fn render_voice_badge(state: &VoiceInputState) -> String {
    match state {
        VoiceInputState::Idle => "\x1b[2;37m[🎙 Voice: Ready]\x1b[0m".to_string(),
        VoiceInputState::Listening { current_rms_db, .. } => {
            let meter = AudioLevelMeter::render_meter(*current_rms_db, 6);
            format!("\x1b[1;34m[🎙 Listening {}\x1b[1;34m]\x1b[0m", meter)
        }
        VoiceInputState::Recording {
            current_rms_db,
            duration_ms,
            ..
        } => {
            let secs = *duration_ms as f64 / 1000.0;
            let meter = AudioLevelMeter::render_meter(*current_rms_db, 6);
            format!("\x1b[1;31m[● REC {:.1}s {}\x1b[1;31m]\x1b[0m", secs, meter)
        }
        VoiceInputState::Transcribing {
            audio_duration_secs,
            ..
        } => {
            format!(
                "\x1b[1;33m[⚡ Transcribing ({:.1}s)...]\x1b[0m",
                audio_duration_secs
            )
        }
        VoiceInputState::Completed { text, .. } => {
            let truncated = if text.len() > 24 {
                format!("{}...", &text[..21])
            } else {
                text.clone()
            };
            format!("\x1b[1;32m[✓ \"{}\"]\x1b[0m", truncated)
        }
        VoiceInputState::Failed { error } => {
            let truncated = if error.len() > 20 {
                format!("{}...", &error[..17])
            } else {
                error.clone()
            };
            format!("\x1b[1;31m[✗ Voice Err: {}]\x1b[0m", truncated)
        }
        VoiceInputState::Cancelled => "\x1b[2;37m[⊘ Voice Cancelled]\x1b[0m".to_string(),
    }
}

/// Renders a full banner for active voice recording with live duration and level meter.
pub fn render_recording_banner(state: &VoiceInputState, width: usize) -> String {
    let target_width = width.max(30);
    match state {
        VoiceInputState::Recording {
            duration_ms,
            current_rms_db,
            ..
        } => {
            let secs = *duration_ms as f64 / 1000.0;
            let meter =
                AudioLevelMeter::render_meter(*current_rms_db, target_width.saturating_sub(20));
            format!(
                "\x1b[1;37;41m ● RECORDING \x1b[0m \x1b[1;31m{:.1}s\x1b[0m [{}] \x1b[2;37m(Press Enter to finish, Ctrl+C to cancel)\x1b[0m",
                secs, meter
            )
        }
        VoiceInputState::Listening { current_rms_db, .. } => {
            let meter =
                AudioLevelMeter::render_meter(*current_rms_db, target_width.saturating_sub(20));
            format!(
                "\x1b[1;37;44m 🎙 LISTENING \x1b[0m [{}] \x1b[2;37m(Speak to begin recording)\x1b[0m",
                meter
            )
        }
        VoiceInputState::Transcribing {
            audio_duration_secs,
            ..
        } => {
            format!(
                "\x1b[1;30;43m ⚡ TRANSCRIBING \x1b[0m \x1b[33mProcessing {:.1}s audio...\x1b[0m",
                audio_duration_secs
            )
        }
        _ => render_voice_badge(state),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_extensions_and_mimes() {
        assert_eq!(AudioFormat::Wav.file_extension(), "wav");
        assert_eq!(AudioFormat::Wav.mime_type(), "audio/wav");

        assert_eq!(AudioFormat::Mp3.file_extension(), "mp3");
        assert_eq!(AudioFormat::Mp3.mime_type(), "audio/mpeg");

        assert_eq!(AudioFormat::Flac.file_extension(), "flac");
        assert_eq!(AudioFormat::from_mime_or_ext("wav"), Some(AudioFormat::Wav));
        assert_eq!(
            AudioFormat::from_mime_or_ext(".mp3"),
            Some(AudioFormat::Mp3)
        );
        assert_eq!(
            AudioFormat::from_mime_or_ext("audio/ogg"),
            Some(AudioFormat::Ogg)
        );
        assert_eq!(AudioFormat::from_mime_or_ext("unknown"), None);
    }

    #[test]
    fn test_audio_buffer_creation_and_metrics() {
        let mut buf = AudioBuffer::new(16000, 1);
        assert_eq!(buf.sample_rate, 16000);
        assert_eq!(buf.channels, 1);
        assert!(buf.is_empty());
        assert_eq!(buf.duration_secs(), 0.0);

        // Add 16000 samples (1 second of audio at 16 kHz)
        let samples = vec![1000i16; 16000];
        buf.push_samples(&samples);

        assert_eq!(buf.len(), 16000);
        assert_eq!(buf.duration_secs(), 1.0);
        assert_eq!(buf.duration_ms(), 1000);

        let rms = buf.rms();
        assert!(rms > 0.0 && rms < 1.0);
        let peak = buf.peak();
        assert!((peak - (1000.0 / 32768.0)).abs() < 1e-4);

        let db = buf.rms_db();
        assert!(db < 0.0 && db > -96.0);
    }

    #[test]
    fn test_audio_buffer_sine_wave_and_silence() {
        let sine = AudioBuffer::generate_sine_wave(440.0, 0.5, 16000, 0.8);
        assert_eq!(sine.len(), 8000);
        assert_eq!(sine.duration_secs(), 0.5);
        assert!(sine.peak() > 0.7);
        assert!(!sine.is_silent(-40.0));

        let silence = AudioBuffer::generate_silence(0.5, 16000);
        assert_eq!(silence.len(), 8000);
        assert_eq!(silence.rms(), 0.0);
        assert_eq!(silence.rms_db(), -96.0);
        assert!(silence.is_silent(-50.0));
    }

    #[test]
    fn test_pure_rust_wav_serialization_roundtrip() {
        // Create an audio buffer with synthetic samples
        let original = AudioBuffer::generate_sine_wave(880.0, 0.25, 16000, 0.5);
        let wav_bytes = original.to_wav_bytes();

        // Standard WAV header is 44 bytes + data
        assert!(wav_bytes.len() >= 44);
        assert_eq!(&wav_bytes[0..4], b"RIFF");
        assert_eq!(&wav_bytes[8..12], b"WAVE");
        assert_eq!(&wav_bytes[12..16], b"fmt ");

        // Parse back from bytes
        let parsed =
            AudioBuffer::from_wav_bytes(&wav_bytes).expect("Failed to parse generated WAV");
        assert_eq!(parsed.sample_rate, original.sample_rate);
        assert_eq!(parsed.channels, original.channels);
        assert_eq!(parsed.len(), original.len());

        for (orig, dec) in original.samples.iter().zip(parsed.samples.iter()) {
            assert_eq!(orig, dec);
        }
    }

    #[test]
    fn test_vad_detector_transitions() {
        let config = VadConfig {
            threshold_db: -40.0,
            silence_timeout_ms: 300,
            min_speech_duration_ms: 100,
        };
        let mut vad = VadDetector::new(config);
        assert_eq!(vad.state(), VadState::Listening);

        // Feed silence -> should stay Listening
        let silence_chunk = AudioBuffer::generate_silence(0.1, 16000);
        let s1 = vad.process_chunk(&silence_chunk);
        assert_eq!(s1, VadState::Listening);

        // Feed active speech -> should transition to Speaking
        let speech_chunk = AudioBuffer::generate_sine_wave(440.0, 0.15, 16000, 0.8);
        let s2 = vad.process_chunk(&speech_chunk);
        assert_eq!(s2, VadState::Speaking);
        assert!(vad.has_speech_started());

        // Feed silence chunks totaling > 300ms -> should transition to SpeechEnded
        vad.process_chunk(&AudioBuffer::generate_silence(0.15, 16000));
        let s3 = vad.process_chunk(&AudioBuffer::generate_silence(0.20, 16000));
        assert_eq!(s3, VadState::SpeechEnded);
    }

    #[test]
    fn test_audio_level_meter_rendering() {
        let glyph_silent = AudioLevelMeter::unicode_glyph(0.0);
        assert_eq!(glyph_silent, ' ');

        let glyph_loud = AudioLevelMeter::unicode_glyph(1.0);
        assert_eq!(glyph_loud, '█');

        let meter = AudioLevelMeter::render_meter(-20.0, 10);
        assert_eq!(meter.chars().count(), 10);

        let sparkline = AudioLevelMeter::render_sparkline(&[0.1, 0.3, 0.6, 0.9], 10);
        assert_eq!(sparkline.chars().count(), 4);
    }

    #[test]
    fn test_voice_config_defaults_and_env() {
        let config = VoiceConfig::new(true)
            .with_provider(SttProvider::Groq)
            .with_model("whisper-large-v3")
            .with_api_key("gsk_test_123")
            .with_language("en");

        assert!(config.enabled);
        assert_eq!(config.provider, SttProvider::Groq);
        assert_eq!(config.model, "whisper-large-v3");
        assert_eq!(config.api_key.as_deref(), Some("gsk_test_123"));
        assert_eq!(config.language.as_deref(), Some("en"));
        assert_eq!(
            config.effective_endpoint(),
            Some("https://api.groq.com/openai/v1/audio/transcriptions".to_string())
        );
    }

    #[test]
    fn test_multipart_builder() {
        let mut builder = MultipartBuilder::new();
        builder.add_text_field("model", "whisper-1");
        builder.add_file_field("file", "audio.wav", "audio/wav", b"RIFFTESTDATA");
        let (content_type, body) = builder.finish();

        assert!(content_type.starts_with("multipart/form-data; boundary="));
        let body_str = String::from_utf8_lossy(&body);
        assert!(body_str.contains("name=\"model\""));
        assert!(body_str.contains("whisper-1"));
        assert!(body_str.contains("filename=\"audio.wav\""));
    }

    #[tokio::test]
    async fn test_mock_stt_adapter_transcription() {
        let adapter = MockSttAdapter::new("Refactor the parser module please.");
        assert!(adapter.is_available());
        assert_eq!(adapter.provider_name(), "mock");

        let buffer = AudioBuffer::generate_sine_wave(440.0, 0.5, 16000, 0.5);
        let config = VoiceConfig::new(true);

        let result = adapter
            .transcribe_buffer(&buffer, &config)
            .await
            .expect("Mock transcription should succeed");

        assert_eq!(result.text, "Refactor the parser module please.");
        assert_eq!(result.provider, "mock");
        assert_eq!(result.language.as_deref(), Some("en"));
        assert!(!result.segments.is_empty());
    }

    #[tokio::test]
    async fn test_voice_session_lifecycle() {
        let config = VoiceConfig::new(true).with_silence_timeout_ms(200);
        let adapter = Box::new(MockSttAdapter::new("Search for error logs"));
        let mut session = VoiceSession::new(config, adapter);

        assert_eq!(*session.state(), VoiceInputState::Idle);

        session.start();
        assert!(matches!(session.state(), VoiceInputState::Listening { .. }));

        // Feed speech samples
        let speech = AudioBuffer::generate_sine_wave(440.0, 0.2, 16000, 0.8);
        session.feed_samples(&speech.samples);
        assert!(matches!(session.state(), VoiceInputState::Recording { .. }));

        // Finalize and transcribe
        let result = session
            .finish_and_transcribe()
            .await
            .expect("Session transcription failed");

        assert_eq!(result.text, "Search for error logs");
        assert!(matches!(session.state(), VoiceInputState::Completed { .. }));
    }

    #[test]
    fn test_ui_badges() {
        let idle = VoiceInputState::Idle;
        assert!(render_voice_badge(&idle).contains("Voice: Ready"));

        let rec = VoiceInputState::Recording {
            duration_ms: 1500,
            sample_count: 24000,
            current_rms_db: -18.0,
        };
        assert!(render_voice_badge(&rec).contains("REC 1.5s"));

        let completed = VoiceInputState::Completed {
            text: "Hello world".to_string(),
            duration_ms: 1500,
            audio_duration_secs: 1.5,
        };
        assert!(render_voice_badge(&completed).contains("Hello world"));
    }
}
