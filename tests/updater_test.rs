use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

use fusion::agent::updater::{
    current_target, is_newer_version, verify_sha256, SimpleSemVer, UpdateState, UpdateStatus,
};

#[test]
fn test_current_target_detection() {
    let target = current_target();
    assert!(!target.is_empty());
    assert_ne!(target, "unknown");
    assert!(
        target == "aarch64-apple-darwin"
            || target == "x86_64-apple-darwin"
            || target == "aarch64-unknown-linux-musl"
            || target == "x86_64-unknown-linux-musl"
            || target == "aarch64-linux-android"
            || target == "x86_64-pc-windows-msvc"
    );
}

#[test]
fn test_semver_precedence_and_prereleases() {
    assert!(is_newer_version("v2.0.0-alpha.3", "v2.0.0-alpha.2"));
    assert!(is_newer_version("v2.0.0-beta.1", "v2.0.0-alpha.5"));
    assert!(is_newer_version("v2.0.0-rc.1", "v2.0.0-beta.2"));
    assert!(is_newer_version("v2.0.0", "v2.0.0-rc.3"));
    assert!(is_newer_version("v2.1.0", "v2.0.0"));
    assert!(is_newer_version("v3.0.0", "v2.99.99"));

    assert!(!is_newer_version("v2.0.0-alpha.2", "v2.0.0-alpha.2"));
    assert!(!is_newer_version("v2.0.0-alpha.1", "v2.0.0-alpha.2"));
    assert!(!is_newer_version("v1.9.9", "v2.0.0-alpha.1"));
}

#[test]
fn test_sha256_hash_validation() {
    let data = b"Fusion binary test payload";
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let hex = format!("{:x}", hasher.finalize());

    assert!(verify_sha256(data, &hex));
    // Test with sha256sum line output: "<hash>  <filename>"
    assert!(verify_sha256(
        data,
        &format!("{}  fusion-v2.0.0-alpha.2.tar.gz", hex)
    ));
    // Bad hash
    assert!(!verify_sha256(data, "abcdef1234567890"));
}

#[test]
fn test_simulated_atomic_swap() {
    let dir = tempdir().unwrap();
    let bin_path = dir.path().join("fusion");
    let pending_path = dir.path().join("pending_fusion");
    let old_backup = dir.path().join("fusion.old");

    // 1. Initial "running" binary
    fs::write(&bin_path, b"version 2.0.0-alpha.2").unwrap();
    assert_eq!(fs::read(&bin_path).unwrap(), b"version 2.0.0-alpha.2");

    // 2. Pending staged binary
    fs::write(&pending_path, b"version 2.0.0-alpha.3").unwrap();

    // 3. Perform atomic swap sequence
    fs::rename(&bin_path, &old_backup).expect("rename to old");
    fs::rename(&pending_path, &bin_path).expect("rename pending to active");
    let _ = fs::remove_file(&old_backup);

    // 4. Verify updated binary is now active
    assert_eq!(fs::read(&bin_path).unwrap(), b"version 2.0.0-alpha.3");
    assert!(!pending_path.exists());
    assert!(!old_backup.exists());
}
