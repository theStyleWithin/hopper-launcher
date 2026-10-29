//! Local hardware detection. Nothing here leaves the machine.

use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct Device {
    pub os: String,
    pub arch: String,
    pub ram_gb: Option<f64>,
    pub gpus: Vec<String>,
}

pub fn detect() -> Device {
    let arch = match std::env::consts::ARCH {
        "arm64" => "aarch64",
        other => other,
    };
    Device {
        os: std::env::consts::OS.to_string(),
        arch: arch.to_string(),
        ram_gb: ram_gb(),
        gpus: gpus(),
    }
}

#[cfg(target_os = "linux")]
fn ram_gb() -> Option<f64> {
    parse_mem_total_kb(&std::fs::read_to_string("/proc/meminfo").ok()?)
        .map(|kb| (kb as f64 / 1024.0 / 1024.0 * 10.0).round() / 10.0)
}

#[cfg(not(target_os = "linux"))]
fn ram_gb() -> Option<f64> {
    None
}

#[cfg(target_os = "linux")]
fn gpus() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return out };
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_card = name.strip_prefix("card").map_or(false, |n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        if !is_card {
            continue;
        }
        if let Ok(v) = std::fs::read_to_string(entry.path().join("device/vendor")) {
            let label = vendor_name(v.trim());
            if !out.iter().any(|g| g == label) {
                out.push(label.to_string());
            }
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn gpus() -> Vec<String> {
    Vec::new()
}

#[allow(dead_code)]
fn parse_mem_total_kb(meminfo: &str) -> Option<u64> {
    meminfo
        .lines()
        .find(|l| l.starts_with("MemTotal:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[allow(dead_code)]
fn vendor_name(id: &str) -> &'static str {
    match id.to_lowercase().as_str() {
        "0x10de" => "NVIDIA",
        "0x1002" => "AMD",
        "0x8086" => "Intel",
        _ => "Other GPU",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_memtotal() {
        assert_eq!(parse_mem_total_kb("MemTotal:       16384000 kB\nMemFree: 1 kB\n"), Some(16384000));
        assert_eq!(parse_mem_total_kb("nothing here"), None);
    }

    #[test]
    fn names_known_gpu_vendors() {
        assert_eq!(vendor_name("0x10DE"), "NVIDIA");
        assert_eq!(vendor_name("0x1002"), "AMD");
        assert_eq!(vendor_name("0xabcd"), "Other GPU");
    }
}
