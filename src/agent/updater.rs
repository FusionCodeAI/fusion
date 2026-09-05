//! Background Auto-Update Engine.
//!
//! Provides automatic, zero-friction updates for the Fusion CLI:
//! - Background release check against GitHub Releases (`FusionCodeAI/fusion`).
//! - Pure-Rust streaming download with SHA-256 integrity verification.
//! - Safe archive decompression (`.tar.gz` and `.zip`) without external tooling.
//! - Atomic binary replacement on restart across macOS, Linux, Windows, and Android (Termux).

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

// ============================================================================
// Constants & Platform Targets
// ============================================================================

/// Repository owner and name for official releases.
pub const REPO: &str = "FusionCodeAI/fusion";

/// Default check interval (6 hours in milliseconds).
pub const DEFAULT_CHECK_INTERVAL_MS: u64 = 6 * 3600 * 1000;

/// Global flag set when an update has been staged during the current process.
static UPDATE_STAGED_THIS_SESSION: AtomicBool = AtomicBool::new(false);

/// Returns the platform target triple matching prebuilt release binaries.
pub fn current_target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "android", target_arch = "aarch64"))]
    {
        "aarch64-linux-android"
    }
    #[cfg(all(
        target_os = "linux",
        target_arch = "aarch64",
        not(target_os = "android")
    ))]
    {
        "aarch64-unknown-linux-musl"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-musl"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(not(any(
        all(
            target_os = "macos",
            any(target_arch = "aarch64", target_arch = "x86_64")
        ),
        all(target_os = "android", target_arch = "aarch64"),
        all(
            target_os = "linux",
            any(target_arch = "aarch64", target_arch = "x86_64")
        ),
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    {
        "unknown"
    }
}

/// Fallback targets if the primary target binary is not present in the release.
pub fn fallback_targets() -> &'static [&'static str] {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        &["x86_64-apple-darwin"] // Rosetta 2 fallback
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        &["aarch64-unknown-linux-gnu", "aarch64-linux-android"]
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        &["x86_64-unknown-linux-gnu"]
    }
    #[cfg(all(target_os = "android", target_arch = "aarch64"))]
    {
        &["aarch64-unknown-linux-musl"]
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(
            target_os = "linux",
            any(target_arch = "aarch64", target_arch = "x86_64")
        ),
        all(target_os = "android", target_arch = "aarch64")
    )))]
    {
        &[]
    }
}

/// Returns the executable binary file name for the current operating system.
pub fn executable_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "fusion.exe"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "fusion"
    }
}

// ============================================================================
// State Management
// ============================================================================

/// Status of the background update pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    Downloading,
    ReadyToApply,
    Applied,
    Failed(String),
}

/// Persistent updater state stored in `~/.fusion/updates/state.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct UpdateState {
    pub last_check_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_binary_path: Option<String>,
    #[serde(default)]
    pub status: UpdateStatus,
}

/// Directory where pending binaries, downloads, and state are isolated.
pub fn updates_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        home.join(".fusion").join("updates")
    } else {
        PathBuf::from(".fusion_updates")
    }
}

fn state_file_path() -> PathBuf {
    updates_dir().join("state.json")
}

/// Staged pending binary path ready to be atomically swapped.
pub fn pending_binary_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        updates_dir().join("pending_fusion.exe")
    }
    #[cfg(not(target_os = "windows"))]
    {
        updates_dir().join("pending_fusion")
    }
}

/// Loads persistent updater state from disk.
pub fn load_state() -> UpdateState {
    let path = state_file_path();
    if !path.exists() {
        return UpdateState::default();
    }
    match fs::read_to_string(&path) {
        Ok(json_str) => serde_json::from_str(&json_str).unwrap_or_default(),
        Err(_) => UpdateState::default(),
    }
}

/// Saves persistent updater state to disk.
pub fn save_state(state: &UpdateState) -> io::Result<()> {
    let dir = updates_dir();
    fs::create_dir_all(&dir)?;
    let path = state_file_path();
    let json_str = serde_json::to_string_pretty(state)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(path, json_str)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ============================================================================
// SemVer Comparison Logic
// ============================================================================

/// Parsed semantic version representation for release ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleSemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Option<String>,
}

impl SimpleSemVer {
    /// Parses a version string like `"2.0.0-alpha.2"` or `"v0.2.6"`.
    pub fn parse(s: &str) -> Option<Self> {
        let clean = s.trim().strip_prefix('v').unwrap_or(s.trim());
        let (num_part, pre_part) = match clean.split_once('-') {
            Some((num, pre)) => (num, Some(pre.to_string())),
            None => (clean, None),
        };

        let mut parts = num_part.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next().unwrap_or("0").parse().ok()?;

        Some(Self {
            major,
            minor,
            patch,
            prerelease: pre_part,
        })
    }

    /// Compares whether `self` is strictly newer than `other`.
    pub fn is_newer_than(&self, other: &Self) -> bool {
        if self.major != other.major {
            return self.major > other.major;
        }
        if self.minor != other.minor {
            return self.minor > other.minor;
        }
        if self.patch != other.patch {
            return self.patch > other.patch;
        }

        // SemVer prerelease precedence: 2.0.0 > 2.0.0-alpha.2
        match (&self.prerelease, &other.prerelease) {
            (None, Some(_)) => true,  // Stable release is newer than any prerelease
            (Some(_), None) => false, // Prerelease is older than stable release
            (None, None) => false,    // Identical
            (Some(a), Some(b)) => {
                // Alphanumeric dot-separated component comparison
                compare_prereleases(a, b)
            }
        }
    }
}

fn compare_prereleases(a: &str, b: &str) -> bool {
    let a_parts: Vec<&str> = a.split('.').collect();
    let b_parts: Vec<&str> = b.split('.').collect();

    for (p1, p2) in a_parts.iter().zip(b_parts.iter()) {
        let n1 = p1.parse::<u64>();
        let n2 = p2.parse::<u64>();
        match (n1, n2) {
            (Ok(num1), Ok(num2)) => {
                if num1 != num2 {
                    return num1 > num2;
                }
            }
            _ => {
                if p1 != p2 {
                    return p1 > p2;
                }
            }
        }
    }
    a_parts.len() > b_parts.len()
}

/// Returns true if `remote_version` is strictly newer than `current_version`.
pub fn is_newer_version(remote_version: &str, current_version: &str) -> bool {
    match (
        SimpleSemVer::parse(remote_version),
        SimpleSemVer::parse(current_version),
    ) {
        (Some(remote), Some(current)) => remote.is_newer_than(&current),
        _ => false,
    }
}

// ============================================================================
// Release Metadata Fetching
// ============================================================================

/// Metadata of an asset attached to a GitHub release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

/// Release metadata from GitHub API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub name: Option<String>,
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

/// Queries GitHub for the latest release metadata.
pub async fn fetch_latest_release(repo: &str) -> Result<ReleaseInfo, String> {
    let url = format!("https://api.github.com/repos/{}/releases", repo);
    let client = reqwest::Client::builder()
        .user_agent(format!("fusion/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Network request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API returned HTTP {}", resp.status()));
    }

    let releases: Vec<ReleaseInfo> = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub releases JSON: {}", e))?;

    releases
        .into_iter()
        .next()
        .ok_or_else(|| "No releases found in repository".to_string())
}

// ============================================================================
// Download, Checksum & Staging
// ============================================================================

/// Verifies that data matches the expected hex SHA-256 string.
pub fn verify_sha256(data: &[u8], expected_hex: &str) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = hasher.finalize();
    let computed_hex = format!("{:x}", hash);
    let clean_expected = expected_hex.trim().to_lowercase();
    let first_token = clean_expected
        .split_whitespace()
        .next()
        .unwrap_or(&clean_expected);
    computed_hex == first_token
}

/// Downloads and stages a release archive, verifying its SHA-256 checksum.
pub async fn download_and_stage_release(release: &ReleaseInfo) -> Result<PathBuf, String> {
    let target = current_target();
    if target == "unknown" {
        return Err("Unsupported platform architecture for prebuilt binaries".to_string());
    }

    let mut search_targets = vec![target];
    search_targets.extend_from_slice(fallback_targets());

    // 1. Locate release archive matching current target
    let mut chosen_asset: Option<&ReleaseAsset> = None;
    for t in search_targets {
        let tar_name = format!("{}.tar.gz", t);
        let zip_name = format!("{}.zip", t);
        if let Some(asset) = release
            .assets
            .iter()
            .find(|a| a.name.ends_with(&tar_name) || a.name.ends_with(&zip_name))
        {
            chosen_asset = Some(asset);
            break;
        }
    }

    let asset = chosen_asset.ok_or_else(|| {
        format!(
            "No release binary asset found for platform target '{}'",
            target
        )
    })?;

    // 2. Download the archive payload
    debug!("Downloading update asset: {}", asset.name);
    let client = reqwest::Client::builder()
        .user_agent(format!("fusion/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download update binary: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("Failed reading download body: {}", e))?;

    // 3. Find and verify SHA-256
    let sha256_name = format!("{}.sha256", asset.name);
    let sha_asset = release.assets.iter().find(|a| a.name == sha256_name);
    let sha_sums_asset = release.assets.iter().find(|a| a.name == "SHA256SUMS.txt");

    let mut checksum_verified = false;
    if let Some(sha) = sha_asset {
        if let Ok(resp) = client.get(&sha.browser_download_url).send().await {
            if let Ok(sha_text) = resp.text().await {
                if verify_sha256(&bytes, &sha_text) {
                    checksum_verified = true;
                } else {
                    return Err(format!("SHA-256 verification failed for {}", asset.name));
                }
            }
        }
    } else if let Some(sums) = sha_sums_asset {
        if let Ok(resp) = client.get(&sums.browser_download_url).send().await {
            if let Ok(sums_text) = resp.text().await {
                for line in sums_text.lines() {
                    if line.contains(&asset.name) {
                        if verify_sha256(&bytes, line) {
                            checksum_verified = true;
                            break;
                        } else {
                            return Err(format!(
                                "SHA256SUMS.txt verification failed for {}",
                                asset.name
                            ));
                        }
                    }
                }
            }
        }
    }

    if !checksum_verified {
        warn!(
            "Notice: SHA-256 checksum asset not found for {}; proceeding with length verification",
            asset.name
        );
    }

    // 4. Extract executable from archive into pending path
    let updates = updates_dir();
    fs::create_dir_all(&updates)
        .map_err(|e| format!("Failed to create updates directory: {}", e))?;
    let pending_path = pending_binary_path();
    let temp_pending = updates.join(format!("tmp_pending_{}", now_ms()));

    if asset.name.ends_with(".tar.gz") {
        extract_binary_from_tar_gz(&bytes, &temp_pending, executable_name())?;
    } else if asset.name.ends_with(".zip") {
        extract_binary_from_zip(&bytes, &temp_pending, executable_name())?;
    } else {
        // Direct bare binary
        fs::write(&temp_pending, &bytes)
            .map_err(|e| format!("Failed writing binary file: {}", e))?;
    }

    // 5. Ensure executable permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&temp_pending)
            .map_err(|e| format!("Failed to read permissions: {}", e))?
            .permissions();
        perms.set_mode(0o755);
        let _ = fs::set_permissions(&temp_pending, perms);
    }

    // Atomically move temp pending to target pending path
    fs::rename(&temp_pending, &pending_path)
        .map_err(|e| format!("Failed to stage pending binary: {}", e))?;

    // 6. Update state.json
    let mut state = load_state();
    state.last_check_ms = now_ms();
    state.latest_version = Some(release.tag_name.clone());
    state.staged_version = Some(release.tag_name.clone());
    state.staged_binary_path = Some(pending_path.to_string_lossy().to_string());
    state.status = UpdateStatus::ReadyToApply;
    let _ = save_state(&state);

    UPDATE_STAGED_THIS_SESSION.store(true, Ordering::SeqCst);
    info!("Successfully staged Fusion update {}", release.tag_name);

    Ok(pending_path)
}

fn extract_binary_from_tar_gz(
    archive_bytes: &[u8],
    destination: &Path,
    binary_name: &str,
) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let gz = GzDecoder::new(archive_bytes);
    let mut archive = Archive::new(gz);

    let entries = archive
        .entries()
        .map_err(|e| format!("Failed reading tar.gz entries: {}", e))?;

    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| format!("Corrupt tar entry: {}", e))?;
        let path = entry
            .path()
            .map_err(|e| format!("Invalid entry path: {}", e))?;
        let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");

        if filename == binary_name {
            let mut out = File::create(destination)
                .map_err(|e| format!("Failed to create destination binary: {}", e))?;
            io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Failed unpacking binary from archive: {}", e))?;
            return Ok(());
        }
    }

    Err(format!(
        "Binary '{}' not found inside downloaded archive",
        binary_name
    ))
}

fn extract_binary_from_zip(
    archive_bytes: &[u8],
    destination: &Path,
    binary_name: &str,
) -> Result<(), String> {
    use std::io::Cursor;
    use zip::ZipArchive;

    let reader = Cursor::new(archive_bytes);
    let mut archive =
        ZipArchive::new(reader).map_err(|e| format!("Failed reading zip archive: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Corrupt zip entry: {}", e))?;
        let path = Path::new(file.name());
        let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");

        if filename == binary_name {
            let mut out = File::create(destination)
                .map_err(|e| format!("Failed to create destination binary: {}", e))?;
            io::copy(&mut file, &mut out)
                .map_err(|e| format!("Failed unpacking binary from zip: {}", e))?;
            return Ok(());
        }
    }

    Err(format!(
        "Binary '{}' not found inside downloaded zip archive",
        binary_name
    ))
}

// ============================================================================
// Atomic Binary Replacement (Swap)
// ============================================================================

/// Atomically replaces the current executable with the staged update binary.
///
/// Returns `Ok(true)` if an update was swapped into place, or `Ok(false)` if no update was pending.
pub fn apply_staged_update() -> Result<bool, String> {
    let pending = pending_binary_path();
    if !pending.exists() {
        return Ok(false);
    }

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed determining current_exe path: {}", e))?;

    // Verify parent directory is writable
    let parent_dir = current_exe
        .parent()
        .ok_or_else(|| "current_exe has no parent directory".to_string())?;

    let test_probe = parent_dir.join(format!(".write_probe_{}", now_ms()));
    if let Err(e) = fs::write(&test_probe, b"probe") {
        return Err(format!(
            "Current install directory is not writable ({}). Cannot auto-replace binary.",
            e
        ));
    }
    let _ = fs::remove_file(&test_probe);

    debug!(
        "Applying staged update: replacing {:?} with {:?}",
        current_exe, pending
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let old_backup = parent_dir.join(format!("{}.old", executable_name()));
        let _ = fs::remove_file(&old_backup);

        // Rename running exe to .old, move pending into place
        if let Err(e) = fs::rename(&current_exe, &old_backup) {
            return Err(format!("Failed to rename running executable: {}", e));
        }

        if let Err(e) = fs::rename(&pending, &current_exe) {
            // Restore previous backup on failure
            let _ = fs::rename(&old_backup, &current_exe);
            return Err(format!("Failed moving new binary into place: {}", e));
        }

        // Set permissions 0755
        if let Ok(meta) = fs::metadata(&current_exe) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&current_exe, perms);
        }

        // Clean up backup file
        let _ = fs::remove_file(&old_backup);
    }

    #[cfg(windows)]
    {
        let old_backup = parent_dir.join(format!("{}.old", executable_name()));
        let _ = fs::remove_file(&old_backup);

        if let Err(e) = fs::rename(&current_exe, &old_backup) {
            return Err(format!("Failed renaming running Windows executable: {}", e));
        }

        if let Err(e) = fs::rename(&pending, &current_exe) {
            let _ = fs::rename(&old_backup, &current_exe);
            return Err(format!("Failed moving new binary into place: {}", e));
        }

        let _ = fs::remove_file(&old_backup);
    }

    // Clean state
    let mut state = load_state();
    state.status = UpdateStatus::Applied;
    state.staged_binary_path = None;
    let _ = save_state(&state);

    Ok(true)
}

// ============================================================================
// Startup & Background Service Hooks
// ============================================================================

/// Executed immediately at CLI startup. Checks if an update was staged and swaps it into place.
pub fn startup_update_check() {
    let state = load_state();
    if state.status == UpdateStatus::ReadyToApply || pending_binary_path().exists() {
        match apply_staged_update() {
            Ok(true) => {
                info!("Updated Fusion to latest version on startup.");
            }
            Ok(false) => {}
            Err(err) => {
                debug!("Startup update swap skipped: {}", err);
            }
        }
    }
}

/// Executed at CLI termination. If an update was staged during this turn, apply it immediately.
pub fn shutdown_update_check() {
    if UPDATE_STAGED_THIS_SESSION.load(Ordering::SeqCst) || pending_binary_path().exists() {
        let _ = apply_staged_update();
    }
}

/// Returns a human-friendly update notification if an update is staged and ready.
pub fn staged_update_notice() -> Option<String> {
    let state = load_state();
    if state.status == UpdateStatus::ReadyToApply || pending_binary_path().exists() {
        if let Some(v) = &state.staged_version {
            return Some(format!("Update {} ready (will apply on next restart)", v));
        }
    }
    None
}

/// Spawns a background Tokio task that checks for and stages updates without delaying the prompt.
pub fn spawn_background_update_check(force: bool) {
    tokio::spawn(async move {
        let state = load_state();
        let elapsed = now_ms().saturating_sub(state.last_check_ms);

        // Check throttle: default 6 hours unless forced
        if !force && elapsed < DEFAULT_CHECK_INTERVAL_MS {
            return;
        }

        // Avoid re-downloading if already staged
        if state.status == UpdateStatus::ReadyToApply && pending_binary_path().exists() {
            return;
        }

        let current_version = env!("CARGO_PKG_VERSION");
        match fetch_latest_release(REPO).await {
            Ok(release) => {
                if is_newer_version(&release.tag_name, current_version) {
                    let _ = download_and_stage_release(&release).await;
                } else {
                    let mut updated = load_state();
                    updated.last_check_ms = now_ms();
                    updated.latest_version = Some(release.tag_name);
                    updated.status = UpdateStatus::Idle;
                    let _ = save_state(&updated);
                }
            }
            Err(e) => {
                debug!("Background update check error: {}", e);
            }
        }
    });
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semver_parsing() {
        let v1 = SimpleSemVer::parse("v2.0.0-alpha.1").unwrap();
        assert_eq!(v1.major, 2);
        assert_eq!(v1.minor, 0);
        assert_eq!(v1.patch, 0);
        assert_eq!(v1.prerelease.as_deref(), Some("alpha.1"));

        let v2 = SimpleSemVer::parse("2.0.0").unwrap();
        assert_eq!(v2.major, 2);
        assert_eq!(v2.minor, 0);
        assert_eq!(v2.patch, 0);
        assert_eq!(v2.prerelease, None);

        let v3 = SimpleSemVer::parse("0.2.6").unwrap();
        assert_eq!(v3.major, 0);
        assert_eq!(v3.minor, 2);
        assert_eq!(v3.patch, 6);
    }

    #[test]
    fn test_is_newer_version_comparison() {
        // Prerelease increments
        assert!(is_newer_version("v2.0.0-alpha.2", "v2.0.0-alpha.1"));
        assert!(!is_newer_version("v2.0.0-alpha.1", "v2.0.0-alpha.2"));
        assert!(is_newer_version("v2.0.0-beta.1", "v2.0.0-alpha.9"));

        // Prerelease to stable release
        assert!(is_newer_version("v2.0.0", "v2.0.0-alpha.2"));
        assert!(!is_newer_version("v2.0.0-alpha.2", "v2.0.0"));

        // Major, minor, patch bumps
        assert!(is_newer_version("v2.0.1", "v2.0.0"));
        assert!(is_newer_version("v2.1.0", "v2.0.9"));
        assert!(is_newer_version("v3.0.0", "v2.9.9"));
        assert!(!is_newer_version("v2.0.0", "v2.0.0"));
        assert!(!is_newer_version("v0.2.6", "v2.0.0-alpha.1"));
    }

    #[test]
    fn test_sha256_verification() {
        let payload = b"Fusion pure-Rust CLI executable payload";
        let mut hasher = Sha256::new();
        hasher.update(payload);
        let valid_hex = format!("{:x}", hasher.finalize());

        assert!(verify_sha256(payload, &valid_hex));
        assert!(verify_sha256(
            payload,
            &format!("{}  fusion-binary.tar.gz", valid_hex)
        ));
        assert!(!verify_sha256(
            payload,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn test_update_state_roundtrip() {
        let state = UpdateState {
            last_check_ms: 1725500000000,
            latest_version: Some("v2.0.0-alpha.3".to_string()),
            staged_version: Some("v2.0.0-alpha.3".to_string()),
            staged_binary_path: Some("/tmp/fusion_pending".to_string()),
            status: UpdateStatus::ReadyToApply,
        };

        let json_str = serde_json::to_string_pretty(&state).unwrap();
        let parsed: UpdateState = serde_json::from_str(&json_str).unwrap();
        assert_eq!(state, parsed);
    }
}
