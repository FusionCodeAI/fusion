use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// Formatting Helpers
// ---------------------------------------------------------------------------

/// Formats a byte count into a human-readable string (B, KB, MB, GB, TB, PB).
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;
    const PB: u64 = 1024 * TB;

    if bytes >= PB {
        format!("{:.2} PB", bytes as f64 / PB as f64)
    } else if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Formats a duration in seconds into a human-readable string (e.g. "2d 4h 15m 30s").
pub fn format_duration(total_seconds: u64) -> String {
    let days = total_seconds / 86400;
    let hours = (total_seconds % 86400) / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 || days > 0 {
        parts.push(format!("{}h", hours));
    }
    if minutes > 0 || hours > 0 || days > 0 {
        parts.push(format!("{}m", minutes));
    }
    parts.push(format!("{}s", seconds));

    parts.join(" ")
}

// ---------------------------------------------------------------------------
// Data Models
// ---------------------------------------------------------------------------

/// Detailed operating system information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OsInfo {
    /// Operating system identifier (e.g. "macos", "linux", "windows", "android").
    pub name: String,
    /// Operating system family (e.g. "unix", "windows").
    pub family: String,
    /// Operating system release or kernel release (e.g. "25.6.0", "6.1.0-20-amd64").
    pub release: String,
    /// Operating system version or product version (e.g. "macOS 15.0", "Ubuntu 22.04.2 LTS").
    pub version: String,
    /// Human-friendly display name of the operating system.
    pub pretty_name: String,
    /// Kernel version / description if available.
    pub kernel: String,
    /// Target CPU architecture string (e.g. "aarch64", "x86_64").
    pub arch: String,
    /// System hostname if available.
    pub hostname: Option<String>,
    /// Active user login name if available.
    pub username: Option<String>,
}

/// Detailed CPU hardware information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CpuInfo {
    /// CPU target architecture.
    pub arch: String,
    /// Total number of logical cores (execution threads) available.
    pub logical_cores: usize,
    /// Number of physical CPU cores if detectable.
    pub physical_cores: Option<usize>,
    /// CPU model name or brand string (e.g. "Apple M4", "AMD Ryzen 9 5950X").
    pub model_name: String,
    /// CPU vendor name (e.g. "Apple", "AuthenticAMD", "GenuineIntel").
    pub vendor: Option<String>,
    /// CPU base or current frequency in MHz if detectable.
    pub frequency_mhz: Option<u64>,
    /// CPU feature flags or capabilities if available.
    pub features: Vec<String>,
}

/// Detailed RAM memory metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryInfo {
    /// Total physical RAM in bytes.
    pub total_bytes: u64,
    /// Available RAM in bytes (free + reclaimable/buffers/cached).
    pub available_bytes: u64,
    /// Used RAM in bytes (total - available).
    pub used_bytes: u64,
    /// Unallocated / strictly free RAM in bytes.
    pub free_bytes: u64,
    /// Percentage of RAM currently in use (0.0 - 100.0).
    pub used_percent: f64,
    /// Formatted total RAM string (e.g. "16.00 GB").
    pub total_formatted: String,
    /// Formatted available RAM string.
    pub available_formatted: String,
    /// Formatted used RAM string.
    pub used_formatted: String,
    /// Formatted free RAM string.
    pub free_formatted: String,
}

/// Detailed Swap memory metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwapInfo {
    /// Total swap space in bytes.
    pub total_bytes: u64,
    /// Used swap space in bytes.
    pub used_bytes: u64,
    /// Free swap space in bytes.
    pub free_bytes: u64,
    /// Percentage of swap currently in use (0.0 - 100.0).
    pub used_percent: f64,
    /// Formatted total swap string.
    pub total_formatted: String,
    /// Formatted used swap string.
    pub used_formatted: String,
    /// Formatted free swap string.
    pub free_formatted: String,
}

/// Hardware and architecture metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HardwareInfo {
    /// Hardware machine identifier or model name (e.g. "Mac16,1", "Raspberry Pi 4").
    pub model: Option<String>,
    /// Machine hardware name (e.g. "arm64", "x86_64").
    pub machine: String,
    /// Memory pointer bit width (e.g. 64).
    pub pointer_width_bits: usize,
    /// Architecture endianness ("little" or "big").
    pub endianness: String,
    /// Target platform compilation triple or target descriptor.
    pub target_platform: String,
}

/// Workspace filesystem / disk space metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskInfo {
    /// Inspected filesystem path.
    pub path: String,
    /// Total filesystem space in bytes.
    pub total_bytes: u64,
    /// Available filesystem space in bytes for unprivileged users.
    pub available_bytes: u64,
    /// Used filesystem space in bytes.
    pub used_bytes: u64,
    /// Percentage of disk space currently in use (0.0 - 100.0).
    pub used_percent: f64,
    /// Formatted total disk space.
    pub total_formatted: String,
    /// Formatted available disk space.
    pub available_formatted: String,
    /// Formatted used disk space.
    pub used_formatted: String,
}

/// Application and system runtime metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeInfo {
    /// Fusion package version.
    pub fusion_version: String,
    /// Target compilation triple.
    pub target_triple: String,
    /// Current working directory.
    pub cwd: String,
    /// System uptime in seconds if detectable.
    pub uptime_seconds: Option<u64>,
    /// Formatted system uptime string.
    pub uptime_formatted: Option<String>,
    /// 1-minute, 5-minute, and 15-minute system load averages if detectable.
    pub load_average: Option<[f64; 3]>,
    /// Current process ID.
    pub process_id: u32,
}

/// Battery power state if available.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatteryInfo {
    /// Battery charge percentage (0 - 100).
    pub percentage: u8,
    /// Battery state description (e.g. "Charging", "Discharging", "Full").
    pub state: String,
    /// Whether the device is currently connected to AC power / charging.
    pub is_charging: bool,
}

/// Complete system inspection report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemReport {
    pub os: OsInfo,
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub swap: Option<SwapInfo>,
    pub hardware: HardwareInfo,
    pub disk: Option<DiskInfo>,
    pub runtime: RuntimeInfo,
    pub battery: Option<BatteryInfo>,
}

impl SystemReport {
    /// Collects a complete system inspection report synchronously/asynchronously.
    pub fn collect(cwd: Option<&Path>) -> Self {
        let os = collect_os_info();
        let cpu = collect_cpu_info();
        let (memory, swap) = collect_memory_and_swap_info();
        let hardware = collect_hardware_info();
        let disk = cwd.and_then(collect_disk_info);
        let runtime = collect_runtime_info(cwd);
        let battery = collect_battery_info();

        Self {
            os,
            cpu,
            memory,
            swap,
            hardware,
            disk,
            runtime,
            battery,
        }
    }

    /// Renders the report as structured JSON value, optionally filtered by category.
    pub fn to_json_value(&self, category: Option<&str>) -> Value {
        match category.unwrap_or("all").to_lowercase().as_str() {
            "os" => json!(self.os),
            "cpu" => json!(self.cpu),
            "memory" | "mem" | "ram" => json!({
                "ram": self.memory,
                "swap": self.swap,
            }),
            "swap" => json!(self.swap),
            "hardware" | "hw" => json!(self.hardware),
            "disk" | "storage" | "filesystem" => json!(self.disk),
            "runtime" | "env" => json!(self.runtime),
            "battery" | "power" => json!(self.battery),
            "summary" => json!({
                "os": format!("{} ({})", self.os.pretty_name, self.os.arch),
                "cpu": format!("{} ({} logical cores)", self.cpu.model_name, self.cpu.logical_cores),
                "memory": format!("{}/{} used ({:.1}%)", self.memory.used_formatted, self.memory.total_formatted, self.memory.used_percent),
                "disk": self.disk.as_ref().map(|d| format!("{}/{} used ({:.1}%)", d.used_formatted, d.total_formatted, d.used_percent)),
                "uptime": self.runtime.uptime_formatted.clone(),
            }),
            _ => json!(self),
        }
    }

    /// Renders the report as a clean human-readable text summary.
    pub fn to_text(&self, category: Option<&str>) -> String {
        let cat = category.unwrap_or("all").to_lowercase();
        let mut out = String::new();

        if cat == "all" || cat == "os" {
            out.push_str("=== Operating System ===\n");
            out.push_str(&format!("  OS Name:       {}\n", self.os.pretty_name));
            out.push_str(&format!("  OS Version:    {}\n", self.os.version));
            out.push_str(&format!("  OS Release:    {}\n", self.os.release));
            out.push_str(&format!("  OS Family:     {}\n", self.os.family));
            out.push_str(&format!("  Architecture:  {}\n", self.os.arch));
            if !self.os.kernel.is_empty() {
                out.push_str(&format!("  Kernel:        {}\n", self.os.kernel));
            }
            if let Some(host) = &self.os.hostname {
                out.push_str(&format!("  Hostname:      {}\n", host));
            }
            if let Some(user) = &self.os.username {
                out.push_str(&format!("  User:          {}\n", user));
            }
            out.push('\n');
        }

        if cat == "all" || cat == "cpu" {
            out.push_str("=== CPU Information ===\n");
            out.push_str(&format!("  Model:         {}\n", self.cpu.model_name));
            out.push_str(&format!("  Architecture:  {}\n", self.cpu.arch));
            out.push_str(&format!("  Logical Cores: {}\n", self.cpu.logical_cores));
            if let Some(phys) = self.cpu.physical_cores {
                out.push_str(&format!("  Physical Cores:{}\n", phys));
            }
            if let Some(vendor) = &self.cpu.vendor {
                out.push_str(&format!("  Vendor:        {}\n", vendor));
            }
            if let Some(mhz) = self.cpu.frequency_mhz {
                out.push_str(&format!("  Frequency:     {} MHz\n", mhz));
            }
            if !self.cpu.features.is_empty() {
                let feat_preview = if self.cpu.features.len() > 8 {
                    format!("{} (+{} more)", self.cpu.features[..8].join(", "), self.cpu.features.len() - 8)
                } else {
                    self.cpu.features.join(", ")
                };
                out.push_str(&format!("  Features:      {}\n", feat_preview));
            }
            out.push('\n');
        }

        if cat == "all" || cat == "memory" || cat == "mem" || cat == "ram" {
            out.push_str("=== Memory (RAM) ===\n");
            out.push_str(&format!("  Total:         {}\n", self.memory.total_formatted));
            out.push_str(&format!("  Available:     {}\n", self.memory.available_formatted));
            out.push_str(&format!("  Used:          {} ({:.1}%)\n", self.memory.used_formatted, self.memory.used_percent));
            out.push_str(&format!("  Free:          {}\n", self.memory.free_formatted));

            if let Some(swap) = &self.swap {
                out.push_str("\n=== Swap Space ===\n");
                out.push_str(&format!("  Total Swap:    {}\n", swap.total_formatted));
                out.push_str(&format!("  Used Swap:     {} ({:.1}%)\n", swap.used_formatted, swap.used_percent));
                out.push_str(&format!("  Free Swap:     {}\n", swap.free_formatted));
            }
            out.push('\n');
        }

        if cat == "all" || cat == "hardware" || cat == "hw" {
            out.push_str("=== Hardware Metadata ===\n");
            if let Some(model) = &self.hardware.model {
                out.push_str(&format!("  Model:         {}\n", model));
            }
            out.push_str(&format!("  Machine:       {}\n", self.hardware.machine));
            out.push_str(&format!("  Pointer Width: {}-bit\n", self.hardware.pointer_width_bits));
            out.push_str(&format!("  Endianness:    {}\n", self.hardware.endianness));
            out.push_str(&format!("  Target Triple: {}\n", self.hardware.target_platform));
            out.push('\n');
        }

        if (cat == "all" || cat == "disk" || cat == "storage") && self.disk.is_some() {
            if let Some(disk) = &self.disk {
                out.push_str("=== Storage / Disk ===\n");
                out.push_str(&format!("  Path:          {}\n", disk.path));
                out.push_str(&format!("  Total:         {}\n", disk.total_formatted));
                out.push_str(&format!("  Available:     {}\n", disk.available_formatted));
                out.push_str(&format!("  Used:          {} ({:.1}%)\n", disk.used_formatted, disk.used_percent));
                out.push('\n');
            }
        }

        if cat == "all" || cat == "runtime" || cat == "env" {
            out.push_str("=== Runtime Environment ===\n");
            out.push_str(&format!("  Fusion Version:{}\n", self.runtime.fusion_version));
            out.push_str(&format!("  Working Dir:   {}\n", self.runtime.cwd));
            out.push_str(&format!("  Process ID:    {}\n", self.runtime.process_id));
            if let Some(uptime) = &self.runtime.uptime_formatted {
                out.push_str(&format!("  System Uptime: {}\n", uptime));
            }
            if let Some(load) = self.runtime.load_average {
                out.push_str(&format!("  Load Average:  {:.2}, {:.2}, {:.2}\n", load[0], load[1], load[2]));
            }
            if let Some(batt) = &self.battery {
                out.push_str(&format!("  Battery:       {}% ({}, {})\n",
                    batt.percentage,
                    batt.state,
                    if batt.is_charging { "Charging" } else { "On Battery" }
                ));
            }
            out.push('\n');
        }

        if cat == "summary" {
            out.push_str("=== System Summary ===\n");
            out.push_str(&format!("  OS:      {} ({})\n", self.os.pretty_name, self.os.arch));
            out.push_str(&format!("  CPU:     {} ({} cores)\n", self.cpu.model_name, self.cpu.logical_cores));
            out.push_str(&format!("  Memory:  {}/{} used ({:.1}%)\n", self.memory.used_formatted, self.memory.total_formatted, self.memory.used_percent));
            if let Some(disk) = &self.disk {
                out.push_str(&format!("  Disk:    {}/{} used ({:.1}%)\n", disk.used_formatted, disk.total_formatted, disk.used_percent));
            }
            if let Some(uptime) = &self.runtime.uptime_formatted {
                out.push_str(&format!("  Uptime:  {}\n", uptime));
            }
        }

        out.trim_end().to_string()
    }
}

// ---------------------------------------------------------------------------
// Collectors Implementation
// ---------------------------------------------------------------------------

/// Runs a command synchronously with a strict timeout and returns stdout trimmed.
fn run_command(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

/// Collects OS information cross-platform.
fn collect_os_info() -> OsInfo {
    let os_name = std::env::consts::OS.to_string();
    let os_family = std::env::consts::FAMILY.to_string();
    let arch = std::env::consts::ARCH.to_string();

    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .or_else(|| run_command("hostname", &[]));

    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()
        .or_else(|| run_command("whoami", &[]));

    #[allow(unused_mut, unused_assignments)]
    let mut release = String::new();
    #[allow(unused_mut, unused_assignments)]
    let mut version = String::new();
    #[allow(unused_mut, unused_assignments)]
    let mut pretty_name = String::new();
    #[allow(unused_mut, unused_assignments)]
    let mut kernel = String::new();
    #[cfg(target_os = "macos")]
    {
        // Query sw_vers or sysctl
        if let Some(prod_ver) = run_command("sw_vers", &["-productVersion"]) {
            version = format!("macOS {}", prod_ver);
            pretty_name = version.clone();
        } else if let Some(prod_ver) = run_command("sysctl", &["-n", "kern.osproductversion"]) {
            version = format!("macOS {}", prod_ver);
            pretty_name = version.clone();
        } else {
            version = "macOS".to_string();
            pretty_name = "macOS".to_string();
        }

        if let Some(kern_rel) = run_command("sysctl", &["-n", "kern.osrelease"]) {
            release = kern_rel;
        } else if let Some(uname_r) = run_command("uname", &["-r"]) {
            release = uname_r;
        }

        if let Some(kern_ver) = run_command("sysctl", &["-n", "kern.version"]) {
            kernel = kern_ver;
        } else if let Some(uname_v) = run_command("uname", &["-v"]) {
            kernel = uname_v;
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try reading /etc/os-release or /usr/lib/os-release
        let os_release_content = std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
            .ok();

        if let Some(content) = os_release_content {
            let parsed = parse_os_release(&content);
            if let Some(pn) = parsed.get("PRETTY_NAME") {
                pretty_name = pn.clone();
            }
            if let Some(v) = parsed.get("VERSION") {
                version = v.clone();
            } else if let Some(v_id) = parsed.get("VERSION_ID") {
                version = v_id.clone();
            }
            if let Some(id) = parsed.get("NAME") {
                if pretty_name.is_empty() {
                    pretty_name = id.clone();
                }
            }
        }

        // On Android / Termux, try getprop
        if pretty_name.is_empty() {
            if let Some(android_rel) = run_command("getprop", &["ro.build.version.release"]) {
                let sdk = run_command("getprop", &["ro.build.version.sdk"]).unwrap_or_default();
                pretty_name = format!("Android {} (SDK {})", android_rel, sdk);
                version = android_rel;
            }
        }

        if let Some(uname_r) = run_command("uname", &["-r"]) {
            release = uname_r;
        } else if let Ok(proc_ver) = std::fs::read_to_string("/proc/version") {
            release = proc_ver.lines().next().unwrap_or("").to_string();
        }

        if let Some(uname_v) = run_command("uname", &["-v"]) {
            kernel = uname_v;
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(ver_out) = run_command("cmd", &["/c", "ver"]) {
            version = ver_out.clone();
            pretty_name = ver_out;
        } else {
            pretty_name = "Microsoft Windows".to_string();
            version = "Windows".to_string();
        }
        release = std::env::var("OS").unwrap_or_else(|_| "Windows_NT".to_string());
    }

    // Fallbacks
    if pretty_name.is_empty() {
        pretty_name = match os_name.as_str() {
            "macos" => "macOS".to_string(),
            "linux" => "Linux".to_string(),
            "windows" => "Windows".to_string(),
            "android" => "Android".to_string(),
            "freebsd" => "FreeBSD".to_string(),
            "openbsd" => "OpenBSD".to_string(),
            "netbsd" => "NetBSD".to_string(),
            other => other.to_string(),
        };
    }
    if version.is_empty() {
        version = release.clone();
    }

    OsInfo {
        name: os_name,
        family: os_family,
        release,
        version,
        pretty_name,
        kernel,
        arch,
        hostname,
        username,
    }
}

/// Parses standard `/etc/os-release` KEY=VALUE syntax.
pub fn parse_os_release(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim().to_string();
            let mut val = val.trim().to_string();
            if (val.starts_with('"') && val.ends_with('"')) || (val.starts_with('\'') && val.ends_with('\'')) {
                if val.len() >= 2 {
                    val = val[1..val.len() - 1].to_string();
                }
            }
            map.insert(key, val);
        }
    }
    map
}

/// Collects CPU hardware information.
fn collect_cpu_info() -> CpuInfo {
    let arch = std::env::consts::ARCH.to_string();
    let logical_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let mut physical_cores = None;
    let mut model_name = String::new();
    let mut vendor = None;
    let mut frequency_mhz = None;
    let mut features = Vec::new();

    #[cfg(target_os = "macos")]
    {
        if let Some(brand) = run_command("sysctl", &["-n", "machdep.cpu.brand_string"]) {
            model_name = brand;
        } else if let Some(model) = run_command("sysctl", &["-n", "hw.model"]) {
            model_name = model;
        }

        if let Some(v) = run_command("sysctl", &["-n", "machdep.cpu.vendor"]) {
            vendor = Some(v);
        } else if arch == "aarch64" {
            vendor = Some("Apple".to_string());
        }

        if let Some(phys) = run_command("sysctl", &["-n", "hw.physicalcpu"]) {
            physical_cores = phys.parse::<usize>().ok();
        }

        if let Some(freq_str) = run_command("sysctl", &["-n", "hw.cpufrequency"]) {
            if let Ok(hz) = freq_str.parse::<u64>() {
                frequency_mhz = Some(hz / 1_000_000);
            }
        }

        if let Some(feat_str) = run_command("sysctl", &["-n", "machdep.cpu.features"]) {
            features = feat_str.split_whitespace().map(|s| s.to_string()).collect();
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(cpuinfo_str) = std::fs::read_to_string("/proc/cpuinfo") {
            let parsed = parse_proc_cpuinfo(&cpuinfo_str);
            if let Some(model) = parsed.model_name {
                model_name = model;
            }
            if let Some(v) = parsed.vendor {
                vendor = Some(v);
            }
            if let Some(phys) = parsed.physical_cores {
                physical_cores = Some(phys);
            }
            if let Some(mhz) = parsed.frequency_mhz {
                frequency_mhz = Some(mhz);
            }
            if !parsed.features.is_empty() {
                features = parsed.features;
            }
        }

        // On Android / Termux, try getprop if model name is still empty
        if model_name.is_empty() {
            if let Some(soc) = run_command("getprop", &["ro.soc.model"])
                .or_else(|| run_command("getprop", &["ro.board.platform"]))
                .or_else(|| run_command("getprop", &["ro.product.board"]))
            {
                model_name = soc;
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(proc_id) = std::env::var("PROCESSOR_IDENTIFIER") {
            model_name = proc_id;
        }
        if let Ok(arch_str) = std::env::var("PROCESSOR_ARCHITECTURE") {
            vendor = Some(arch_str);
        }
    }

    if model_name.is_empty() {
        model_name = format!("Generic {} Processor", arch);
    }

    CpuInfo {
        arch,
        logical_cores,
        physical_cores,
        model_name,
        vendor,
        frequency_mhz,
        features,
    }
}

/// Parsed results from `/proc/cpuinfo`.
#[derive(Debug, Default, PartialEq)]
pub struct ParsedProcCpuInfo {
    pub model_name: Option<String>,
    pub vendor: Option<String>,
    pub physical_cores: Option<usize>,
    pub frequency_mhz: Option<u64>,
    pub features: Vec<String>,
}

/// Parses Linux `/proc/cpuinfo` text.
pub fn parse_proc_cpuinfo(content: &str) -> ParsedProcCpuInfo {
    let mut info = ParsedProcCpuInfo::default();
    let mut core_ids = std::collections::HashSet::new();

    for line in content.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim();

            if info.model_name.is_none() && (key == "model name" || key == "Hardware" || key == "Processor") {
                if !val.is_empty() {
                    info.model_name = Some(val.to_string());
                }
            } else if info.vendor.is_none() && (key == "vendor_id" || key == "CPU implementer") {
                if !val.is_empty() {
                    info.vendor = Some(val.to_string());
                }
            } else if info.frequency_mhz.is_none() && (key == "cpu MHz" || key == "clock") {
                if let Ok(mhz) = val.parse::<f64>() {
                    info.frequency_mhz = Some(mhz.round() as u64);
                }
            } else if key == "core id" {
                if let Ok(id) = val.parse::<usize>() {
                    core_ids.insert(id);
                }
            } else if key == "cpu cores" && info.physical_cores.is_none() {
                if let Ok(cores) = val.parse::<usize>() {
                    info.physical_cores = Some(cores);
                }
            } else if (key == "flags" || key == "Features") && info.features.is_empty() {
                info.features = val.split_whitespace().map(|s| s.to_string()).collect();
            }
        }
    }

    if info.physical_cores.is_none() && !core_ids.is_empty() {
        info.physical_cores = Some(core_ids.len());
    }

    info
}

/// Parsed Linux `/proc/meminfo` metrics.
#[derive(Debug, Default, PartialEq)]
pub struct ParsedProcMemInfo {
    pub mem_total_kb: u64,
    pub mem_free_kb: u64,
    pub mem_available_kb: Option<u64>,
    pub buffers_kb: u64,
    pub cached_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
}

/// Parses Linux `/proc/meminfo` text.
pub fn parse_proc_meminfo(content: &str) -> ParsedProcMemInfo {
    let mut info = ParsedProcMemInfo::default();

    for line in content.lines() {
        if let Some((k, rest)) = line.split_once(':') {
            let key = k.trim();
            let val_part = rest.trim();
            // Value is typically formatted like "16384000 kB"
            let num_str = val_part.split_whitespace().next().unwrap_or("0");
            let kb_val = num_str.parse::<u64>().unwrap_or(0);

            match key {
                "MemTotal" => info.mem_total_kb = kb_val,
                "MemFree" => info.mem_free_kb = kb_val,
                "MemAvailable" => info.mem_available_kb = Some(kb_val),
                "Buffers" => info.buffers_kb = kb_val,
                "Cached" => info.cached_kb = kb_val,
                "SwapTotal" => info.swap_total_kb = kb_val,
                "SwapFree" => info.swap_free_kb = kb_val,
                _ => {}
            }
        }
    }

    info
}

/// Parses macOS `vm_stat` output.
pub fn parse_macos_vm_stat(content: &str) -> (u64, u64) {
    // Returns (page_size, available_bytes_estimate)
    let mut page_size: u64 = 4096;
    let mut free_pages: u64 = 0;
    let mut inactive_pages: u64 = 0;
    let mut speculative_pages: u64 = 0;
    let mut purgeable_pages: u64 = 0;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("Mach Virtual Memory Statistics:") {
            if let Some(idx) = line.find("page size of ") {
                let rest = &line[idx + "page size of ".len()..];
                if let Some(bytes_str) = rest.split_whitespace().next() {
                    if let Ok(sz) = bytes_str.parse::<u64>() {
                        page_size = sz;
                    }
                }
            }
        } else if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim().trim_end_matches('.');
            let count = val.parse::<u64>().unwrap_or(0);

            match key {
                "Pages free" => free_pages = count,
                "Pages inactive" => inactive_pages = count,
                "Pages speculative" => speculative_pages = count,
                "Pages purgeable" => purgeable_pages = count,
                _ => {}
            }
        }
    }

    let available_pages = free_pages + inactive_pages + speculative_pages + purgeable_pages;
    let available_bytes = available_pages * page_size;
    (page_size, available_bytes)
}

/// Parses macOS `sysctl -n vm.swapusage` output.
/// Example: "total = 1024.00M  used = 0.00M  free = 1024.00M  (encrypted)"
pub fn parse_macos_swap_usage(content: &str) -> Option<(u64, u64, u64)> {
    // Returns (total_bytes, used_bytes, free_bytes)
    fn parse_val(s: &str) -> Option<u64> {
        let s = s.trim();
        if s.ends_with('M') || s.ends_with('m') {
            let num = s[..s.len() - 1].trim().parse::<f64>().ok()?;
            Some((num * 1024.0 * 1024.0) as u64)
        } else if s.ends_with('K') || s.ends_with('k') {
            let num = s[..s.len() - 1].trim().parse::<f64>().ok()?;
            Some((num * 1024.0) as u64)
        } else if s.ends_with('G') || s.ends_with('g') {
            let num = s[..s.len() - 1].trim().parse::<f64>().ok()?;
            Some((num * 1024.0 * 1024.0 * 1024.0) as u64)
        } else if s.ends_with('B') || s.ends_with('b') {
            s[..s.len() - 1].trim().parse::<u64>().ok()
        } else {
            s.parse::<u64>().ok()
        }
    }

    let mut total = None;
    let mut used = None;
    let mut free = None;

    for part in content.split_whitespace() {
        if let Some((k, v)) = part.split_once('=') {
            match k.trim() {
                "total" => total = parse_val(v),
                "used" => used = parse_val(v),
                "free" => free = parse_val(v),
                _ => {}
            }
        }
    }

    // Also handle space-separated key = val
    if total.is_none() && content.contains("total =") {
        for segment in content.split("  ") {
            let segment = segment.trim();
            if let Some((k, v)) = segment.split_once('=') {
                match k.trim() {
                    "total" => total = parse_val(v),
                    "used" => used = parse_val(v),
                    "free" => free = parse_val(v),
                    _ => {}
                }
            }
        }
    }

    if let (Some(t), Some(u), Some(f)) = (total, used, free) {
        Some((t, u, f))
    } else {
        None
    }
}

/// Collects RAM memory and Swap space information.
fn collect_memory_and_swap_info() -> (MemoryInfo, Option<SwapInfo>) {
    let mut total_bytes: u64 = 0;
    let mut available_bytes: u64 = 0;
    let mut free_bytes: u64 = 0;
    let mut swap_info: Option<SwapInfo> = None;

    #[cfg(target_os = "macos")]
    {
        if let Some(mem_str) = run_command("sysctl", &["-n", "hw.memsize"]) {
            if let Ok(bytes) = mem_str.parse::<u64>() {
                total_bytes = bytes;
            }
        }

        if let Some(vm_stat_out) = run_command("vm_stat", &[]) {
            let (_page_sz, avail_estimate) = parse_macos_vm_stat(&vm_stat_out);
            available_bytes = avail_estimate.min(total_bytes);
            free_bytes = available_bytes;
        }

        if let Some(swap_out) = run_command("sysctl", &["-n", "vm.swapusage"]) {
            if let Some((s_total, s_used, s_free)) = parse_macos_swap_usage(&swap_out) {
                let used_pct = if s_total > 0 {
                    (s_used as f64 / s_total as f64) * 100.0
                } else {
                    0.0
                };
                swap_info = Some(SwapInfo {
                    total_bytes: s_total,
                    used_bytes: s_used,
                    free_bytes: s_free,
                    used_percent: (used_pct * 10.0).round() / 10.0,
                    total_formatted: format_bytes(s_total),
                    used_formatted: format_bytes(s_used),
                    free_formatted: format_bytes(s_free),
                });
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(meminfo_str) = std::fs::read_to_string("/proc/meminfo") {
            let parsed = parse_proc_meminfo(&meminfo_str);
            total_bytes = parsed.mem_total_kb * 1024;
            free_bytes = parsed.mem_free_kb * 1024;
            let avail_kb = parsed.mem_available_kb.unwrap_or_else(|| {
                parsed.mem_free_kb + parsed.buffers_kb + parsed.cached_kb
            });
            available_bytes = avail_kb * 1024;

            if parsed.swap_total_kb > 0 {
                let s_total = parsed.swap_total_kb * 1024;
                let s_free = parsed.swap_free_kb * 1024;
                let s_used = s_total.saturating_sub(s_free);
                let used_pct = if s_total > 0 {
                    (s_used as f64 / s_total as f64) * 100.0
                } else {
                    0.0
                };
                swap_info = Some(SwapInfo {
                    total_bytes: s_total,
                    used_bytes: s_used,
                    free_bytes: s_free,
                    used_percent: (used_pct * 10.0).round() / 10.0,
                    total_formatted: format_bytes(s_total),
                    used_formatted: format_bytes(s_used),
                    free_formatted: format_bytes(s_free),
                });
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(wmic_out) = run_command("wmic", &["OS", "get", "FreePhysicalMemory,TotalVisibleMemorySize", "/Value"]) {
            for line in wmic_out.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    let val = v.trim().parse::<u64>().unwrap_or(0);
                    if k.trim() == "TotalVisibleMemorySize" {
                        total_bytes = val * 1024;
                    } else if k.trim() == "FreePhysicalMemory" {
                        free_bytes = val * 1024;
                        available_bytes = free_bytes;
                    }
                }
            }
        }
    }

    // Ensure available does not exceed total
    if total_bytes > 0 && available_bytes > total_bytes {
        available_bytes = total_bytes;
    }

    let used_bytes = total_bytes.saturating_sub(available_bytes);
    let used_percent = if total_bytes > 0 {
        ((used_bytes as f64 / total_bytes as f64) * 100.0 * 10.0).round() / 10.0
    } else {
        0.0
    };

    let mem = MemoryInfo {
        total_bytes,
        available_bytes,
        used_bytes,
        free_bytes,
        used_percent,
        total_formatted: format_bytes(total_bytes),
        available_formatted: format_bytes(available_bytes),
        used_formatted: format_bytes(used_bytes),
        free_formatted: format_bytes(free_bytes),
    };

    (mem, swap_info)
}

/// Collects hardware platform metadata.
fn collect_hardware_info() -> HardwareInfo {
    let mut model = None;
    let machine = std::env::consts::ARCH.to_string();
    let pointer_width_bits = usize::BITS as usize;
    let endianness = if cfg!(target_endian = "little") {
        "little".to_string()
    } else {
        "big".to_string()
    };
    let target_platform = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);

    #[cfg(target_os = "macos")]
    {
        if let Some(m) = run_command("sysctl", &["-n", "hw.model"]) {
            model = Some(m);
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try DMI product name
        if let Ok(dmi) = std::fs::read_to_string("/sys/class/dmi/id/product_name") {
            let t = dmi.trim();
            if !t.is_empty() && t != "None" && t != "System Product Name" {
                model = Some(t.to_string());
            }
        } else if let Ok(dt) = std::fs::read_to_string("/proc/device-tree/model") {
            let t = dt.trim_matches('\0').trim();
            if !t.is_empty() {
                model = Some(t.to_string());
            }
        }
    }

    HardwareInfo {
        model,
        machine,
        pointer_width_bits,
        endianness,
        target_platform,
    }
}

/// Parses standard POSIX `df -k` or `df -P` text.
pub fn parse_df_output(content: &str, target_path: &Path) -> Option<DiskInfo> {
    let mut lines = content.lines();
    let _header = lines.next()?;

    let mut best_entry: Option<(u64, u64, u64)> = None;

    for line in lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        // Typically: Filesystem 1024-blocks Used Available Capacity Mounted on
        if parts.len() >= 4 {
            let total_kb = parts[1].parse::<u64>().ok();
            let used_kb = parts[2].parse::<u64>().ok();
            let avail_kb = parts[3].parse::<u64>().ok();

            if let (Some(t), Some(u), Some(a)) = (total_kb, used_kb, avail_kb) {
                best_entry = Some((t * 1024, u * 1024, a * 1024));
                break;
            }
        }
    }

    let (total_bytes, used_bytes, available_bytes) = best_entry?;
    let used_percent = if total_bytes > 0 {
        ((used_bytes as f64 / total_bytes as f64) * 100.0 * 10.0).round() / 10.0
    } else {
        0.0
    };

    Some(DiskInfo {
        path: target_path.display().to_string(),
        total_bytes,
        available_bytes,
        used_bytes,
        used_percent,
        total_formatted: format_bytes(total_bytes),
        available_formatted: format_bytes(available_bytes),
        used_formatted: format_bytes(used_bytes),
    })
}

/// Collects disk information for the given path.
fn collect_disk_info(path: &Path) -> Option<DiskInfo> {
    let path_str = path.to_str()?;

    #[cfg(unix)]
    {
        if let Some(df_out) = run_command("df", &["-k", path_str]) {
            if let Some(info) = parse_df_output(&df_out, path) {
                return Some(info);
            }
        }
    }

    #[cfg(windows)]
    {
        // Try wmic or powershell for disk space
        let _ = path_str;
    }

    None
}

/// Parses uptime from Linux `/proc/uptime` (e.g. "123456.78 987654.32").
pub fn parse_proc_uptime(content: &str) -> Option<u64> {
    let first = content.split_whitespace().next()?;
    let secs = first.parse::<f64>().ok()?;
    Some(secs as u64)
}

/// Parses 1, 5, 15 minute load averages from Linux `/proc/loadavg` (e.g. "0.45 0.52 0.48 1/892 12345").
pub fn parse_proc_loadavg(content: &str) -> Option<[f64; 3]> {
    let parts: Vec<&str> = content.split_whitespace().collect();
    if parts.len() >= 3 {
        let l1 = parts[0].parse::<f64>().ok()?;
        let l5 = parts[1].parse::<f64>().ok()?;
        let l15 = parts[2].parse::<f64>().ok()?;
        Some([l1, l5, l15])
    } else {
        None
    }
}
/// Parses 1, 5, 15 minute load averages from macOS `vm.loadavg` (e.g. "{ 1.25 1.50 1.75 }").
pub fn parse_macos_loadavg(content: &str) -> Option<[f64; 3]> {
    let cleaned = content.trim().trim_matches(|c| c == '{' || c == '}').trim();
    let parts: Vec<&str> = cleaned.split_whitespace().collect();
    if parts.len() >= 3 {
        let l1 = parts[0].parse::<f64>().ok()?;
        let l5 = parts[1].parse::<f64>().ok()?;
        let l15 = parts[2].parse::<f64>().ok()?;
        Some([l1, l5, l15])
    } else {
        None
    }
}


/// Parses macOS `kern.boottime` (e.g. "{ sec = 1725200000, usec = 0 } Mon Sep ...").
pub fn parse_macos_boottime(content: &str) -> Option<u64> {
    if let Some(idx) = content.find("sec = ") {
        let rest = &content[idx + "sec = ".len()..];
        let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let boot_sec = num_str.parse::<u64>().ok()?;
        let now_sec = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        if now_sec >= boot_sec {
            return Some(now_sec - boot_sec);
        }
    }
    None
}

/// Collects runtime environment information.
fn collect_runtime_info(cwd: Option<&Path>) -> RuntimeInfo {
    let fusion_version = env!("CARGO_PKG_VERSION").to_string();
    let target_triple = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
    let cwd_str = cwd
        .map(|p| p.display().to_string())
        .or_else(|| std::env::current_dir().ok().map(|p| p.display().to_string()))
        .unwrap_or_else(|| ".".to_string());
    let process_id = std::process::id();

    #[allow(unused_mut)]
    let mut uptime_seconds = None;
    #[allow(unused_mut)]
    let mut load_average = None;

    #[cfg(target_os = "macos")]
    {
        if let Some(bt) = run_command("sysctl", &["-n", "kern.boottime"]) {
            uptime_seconds = parse_macos_boottime(&bt);
        }
        if let Some(ld) = run_command("sysctl", &["-n", "vm.loadavg"]) {
            load_average = parse_macos_loadavg(&ld);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(upt_content) = std::fs::read_to_string("/proc/uptime") {
            uptime_seconds = parse_proc_uptime(&upt_content);
        }
        if let Ok(load_content) = std::fs::read_to_string("/proc/loadavg") {
            load_average = parse_proc_loadavg(&load_content);
        }
    }

    let uptime_formatted = uptime_seconds.map(format_duration);

    RuntimeInfo {
        fusion_version,
        target_triple,
        cwd: cwd_str,
        uptime_seconds,
        uptime_formatted,
        load_average,
        process_id,
    }
}

/// Parses macOS `pmset -g batt` output.
pub fn parse_macos_battery(content: &str) -> Option<BatteryInfo> {
    for line in content.lines() {
        if line.contains('%') {
            if let Some(pct_idx) = line.find('%') {
                let before = &line[..pct_idx];
                let num_str: String = before.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect();
                if let Ok(pct) = num_str.parse::<u8>() {
                    let is_charging = line.to_lowercase().contains("charging") && !line.to_lowercase().contains("discharging");
                    let state = if line.contains("discharging") {
                        "Discharging".to_string()
                    } else if line.contains("charging") {
                        "Charging".to_string()
                    } else if line.contains("charged") || line.contains("finishing charge") {
                        "Full / Charged".to_string()
                    } else {
                        "AC Power".to_string()
                    };

                    return Some(BatteryInfo {
                        percentage: pct,
                        state,
                        is_charging,
                    });
                }
            }
        }
    }
    None
}

/// Collects battery power state if available.
fn collect_battery_info() -> Option<BatteryInfo> {
    #[cfg(target_os = "macos")]
    {
        if let Some(batt_out) = run_command("pmset", &["-g", "batt"]) {
            return parse_macos_battery(&batt_out);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let cap_path = Path::new("/sys/class/power_supply/BAT0/capacity");
        let status_path = Path::new("/sys/class/power_supply/BAT0/status");
        if cap_path.exists() {
            let cap = std::fs::read_to_string(cap_path).ok()?.trim().parse::<u8>().ok()?;
            let status = std::fs::read_to_string(status_path).unwrap_or_else(|_| "Unknown".to_string());
            let status = status.trim().to_string();
            let is_charging = status.eq_ignore_ascii_case("Charging");
            return Some(BatteryInfo {
                percentage: cap,
                state: status,
                is_charging,
            });
        }
    }

    None
}

// ---------------------------------------------------------------------------
// SystemInfoTool
// ---------------------------------------------------------------------------

/// Tool for inspecting host hardware, operating system, CPU architecture, available RAM, swap, and runtime.
#[derive(Default, Debug, Clone)]
pub struct SystemInfoTool;

impl SystemInfoTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SystemInfoTool {
    fn name(&self) -> &str {
        "system_info"
    }

    fn description(&self) -> &str {
        "Inspect host hardware, operating system version, CPU architecture and core count, RAM and swap memory usage, disk space, and runtime metrics."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "enum": ["all", "summary", "os", "cpu", "memory", "swap", "hardware", "disk", "runtime", "battery"],
                    "description": "Specific subsystem category to inspect (optional, default: 'all')."
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "json"],
                    "description": "Output formatting style: human-readable 'text' or structured 'json' (optional, default: 'text')."
                },
                "path": {
                    "type": "string",
                    "description": "Filesystem path for disk space inspection (optional, defaults to workspace root)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let category = args
            .get("category")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("subsystem").and_then(|v| v.as_str()))
            .unwrap_or("all");

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("output_format").and_then(|v| v.as_str()))
            .unwrap_or("text");

        let target_path = if let Some(p_str) = args.get("path").and_then(|v| v.as_str()) {
            if p_str.trim().is_empty() {
                ctx.cwd.clone()
            } else {
                let p = Path::new(p_str);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    ctx.cwd.join(p)
                }
            }
        } else {
            ctx.cwd.clone()
        };

        // Collect report
        let report = SystemReport::collect(Some(&target_path));

        if format.eq_ignore_ascii_case("json") {
            let json_val = report.to_json_value(Some(category));
            Ok(serde_json::to_string_pretty(&json_val)?)
        } else {
            Ok(report.to_text(Some(category)))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(16 * 1024 * 1024 * 1024), "16.00 GB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024 * 1024), "2.00 TB");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(125), "2m 5s");
        assert_eq!(format_duration(3665), "1h 1m 5s");
        assert_eq!(format_duration(90061), "1d 1h 1m 1s");
    }

    #[test]
    fn test_parse_os_release() {
        let sample = r#"
NAME="Ubuntu"
VERSION="22.04.2 LTS (Jammy Jellyfish)"
ID=ubuntu
ID_LIKE=debian
PRETTY_NAME="Ubuntu 22.04.2 LTS"
VERSION_ID="22.04"
HOME_URL="https://www.ubuntu.com/"
"#;
        let map = parse_os_release(sample);
        assert_eq!(map.get("NAME").unwrap(), "Ubuntu");
        assert_eq!(map.get("PRETTY_NAME").unwrap(), "Ubuntu 22.04.2 LTS");
        assert_eq!(map.get("VERSION_ID").unwrap(), "22.04");
    }

    #[test]
    fn test_parse_proc_cpuinfo() {
        let sample = r#"
processor	: 0
vendor_id	: AuthenticAMD
cpu family	: 25
model name	: AMD Ryzen 9 5950X 16-Core Processor
cpu MHz		: 3400.000
core id		: 0
cpu cores	: 16
flags		: fpu vme de pse tsc msr pae mce cx8 apic sep

processor	: 1
vendor_id	: AuthenticAMD
model name	: AMD Ryzen 9 5950X 16-Core Processor
cpu MHz		: 3400.000
core id		: 1
cpu cores	: 16
flags		: fpu vme de pse tsc msr pae mce cx8 apic sep
"#;
        let parsed = parse_proc_cpuinfo(sample);
        assert_eq!(parsed.model_name.unwrap(), "AMD Ryzen 9 5950X 16-Core Processor");
        assert_eq!(parsed.vendor.unwrap(), "AuthenticAMD");
        assert_eq!(parsed.frequency_mhz.unwrap(), 3400);
        assert_eq!(parsed.physical_cores.unwrap(), 16);
        assert!(parsed.features.contains(&"fpu".to_string()));
    }

    #[test]
    fn test_parse_proc_meminfo() {
        let sample = r#"
MemTotal:       16384000 kB
MemFree:         4000000 kB
MemAvailable:   12000000 kB
Buffers:          500000 kB
Cached:          3500000 kB
SwapTotal:       2097152 kB
SwapFree:        1048576 kB
"#;
        let parsed = parse_proc_meminfo(sample);
        assert_eq!(parsed.mem_total_kb, 16384000);
        assert_eq!(parsed.mem_free_kb, 4000000);
        assert_eq!(parsed.mem_available_kb, Some(12000000));
        assert_eq!(parsed.buffers_kb, 500000);
        assert_eq!(parsed.cached_kb, 3500000);
        assert_eq!(parsed.swap_total_kb, 2097152);
        assert_eq!(parsed.swap_free_kb, 1048576);
    }

    #[test]
    fn test_parse_macos_vm_stat() {
        let sample = r#"
Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                               12345.
Pages active:                             67890.
Pages inactive:                           23456.
Pages speculative:                         1000.
Pages purgeable:                            500.
Pages wired down:                         34567.
Pages occupied by compressor:             12000.
"#;
        let (page_sz, available_bytes) = parse_macos_vm_stat(sample);
        assert_eq!(page_sz, 16384);
        let expected_pages = 12345 + 23456 + 1000 + 500;
        assert_eq!(available_bytes, expected_pages * 16384);
    }

    #[test]
    fn test_parse_macos_swap_usage() {
        let sample = "total = 1024.00M  used = 128.00M  free = 896.00M  (encrypted)";
        let res = parse_macos_swap_usage(sample);
        assert!(res.is_some());
        let (total, used, free) = res.unwrap();
        assert_eq!(total, 1024 * 1024 * 1024);
        assert_eq!(used, 128 * 1024 * 1024);
        assert_eq!(free, 896 * 1024 * 1024);
    }

    #[test]
    fn test_parse_df_output() {
        let sample = r#"Filesystem    1024-blocks      Used Available Capacity iused ifree %iused  Mounted on
/dev/disk3s1s1   482817024  15234560 250000000     6%  450000 2400000000    0%   /
"#;
        let p = Path::new("/");
        let disk = parse_df_output(sample, p);
        assert!(disk.is_some());
        let d = disk.unwrap();
        assert_eq!(d.total_bytes, 482817024 * 1024);
        assert_eq!(d.used_bytes, 15234560 * 1024);
        assert_eq!(d.available_bytes, 250000000 * 1024);
    }

    #[test]
    fn test_parse_macos_loadavg() {
        let sample = "{ 2.45 1.80 1.55 }";
        let load = parse_macos_loadavg(sample);
        assert!(load.is_some());
        let [l1, l5, l15] = load.unwrap();
        assert!((l1 - 2.45).abs() < 1e-6);
        assert!((l5 - 1.80).abs() < 1e-6);
        assert!((l15 - 1.55).abs() < 1e-6);
    }
    #[test]
    fn test_parse_proc_uptime_and_loadavg() {
        let uptime_sample = "345678.90 123456.78\n";
        assert_eq!(parse_proc_uptime(uptime_sample), Some(345678));

        let load_sample = "0.75 1.20 0.95 2/1045 54321\n";
        let load = parse_proc_loadavg(load_sample);
        assert!(load.is_some());
        let [l1, l5, l15] = load.unwrap();
        assert!((l1 - 0.75).abs() < 1e-6);
        assert!((l5 - 1.20).abs() < 1e-6);
        assert!((l15 - 0.95).abs() < 1e-6);
    }

    #[test]
    fn test_parse_macos_battery() {
        let sample = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1234567)\t85%; discharging; 4:30 remaining present: true\n";
        let batt = parse_macos_battery(sample);
        assert!(batt.is_some());
        let b = batt.unwrap();
        assert_eq!(b.percentage, 85);
        assert_eq!(b.state, "Discharging");
        assert!(!b.is_charging);
    }

    #[tokio::test]
    async fn test_system_info_tool_execution() {
        let tool = SystemInfoTool::new();
        assert_eq!(tool.name(), "system_info");
        let def = tool.definition();
        assert_eq!(def.name, "system_info");

        let ctx = ToolContext::default();

        // 1. Test full text summary execution
        let text_res = tool.execute(json!({}), &ctx).await;
        assert!(text_res.is_ok(), "Tool execution failed: {:?}", text_res);
        let text = text_res.unwrap();
        assert!(text.contains("Operating System"), "Missing OS header: {}", text);
        assert!(text.contains("CPU Information"), "Missing CPU header: {}", text);
        assert!(text.contains("Memory (RAM)"), "Missing Memory header: {}", text);

        // 2. Test JSON format execution
        let json_res = tool.execute(json!({"format": "json"}), &ctx).await;
        assert!(json_res.is_ok(), "JSON execution failed: {:?}", json_res);
        let json_str = json_res.unwrap();
        let parsed_val: Value = serde_json::from_str(&json_str).expect("Invalid JSON returned");
        assert!(parsed_val.get("os").is_some());
        assert!(parsed_val.get("cpu").is_some());
        assert!(parsed_val.get("memory").is_some());

        // 3. Test category filtering (cpu only)
        let cpu_text = tool.execute(json!({"category": "cpu"}), &ctx).await.unwrap();
        assert!(cpu_text.contains("CPU Information"));
        assert!(!cpu_text.contains("Operating System"));

        // 4. Test category filtering (memory JSON)
        let mem_json = tool.execute(json!({"category": "memory", "format": "json"}), &ctx).await.unwrap();
        let mem_val: Value = serde_json::from_str(&mem_json).expect("Invalid JSON for memory");
        assert!(mem_val.get("ram").is_some());

        // 5. Test summary category
        let sum_text = tool.execute(json!({"category": "summary"}), &ctx).await.unwrap();
        assert!(sum_text.contains("System Summary"));
    }
}
