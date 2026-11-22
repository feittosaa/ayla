use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct Hardware {
    pub ram_total_bytes: u64,
    pub ram_available_bytes: u64,
    pub logical_cpus: usize,
    pub gpus: Vec<Gpu>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Gpu {
    pub vendor: String,
    pub vram_total_bytes: Option<u64>,
    pub vram_available_bytes: Option<u64>,
}

fn memory_value(contents: &str, key: &str) -> u64 {
    contents
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name == key).then(|| value.split_whitespace().next()?.parse::<u64>().ok())?
        })
        .unwrap_or(0)
        .saturating_mul(1024)
}

fn number(path: impl AsRef<Path>) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

pub fn detect() -> Hardware {
    let memory = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut gpus = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("card") || !name[4..].chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let device = entry.path().join("device");
            let vendor = match fs::read_to_string(device.join("vendor"))
                .unwrap_or_default()
                .trim()
            {
                "0x1002" => "AMD",
                "0x10de" => "NVIDIA",
                "0x8086" => "Intel",
                _ => "Outra",
            }
            .to_string();
            let total = number(device.join("mem_info_vram_total"));
            let used = number(device.join("mem_info_vram_used"));
            gpus.push(Gpu {
                vendor,
                vram_total_bytes: total,
                vram_available_bytes: total.zip(used).map(|(t, u)| t.saturating_sub(u)),
            });
        }
    }
    Hardware {
        ram_total_bytes: memory_value(&memory, "MemTotal"),
        ram_available_bytes: memory_value(&memory, "MemAvailable"),
        logical_cpus: std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        gpus,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_memory_without_assuming_line_order() {
        let input = "MemAvailable: 123 kB\nMemTotal: 456 kB\n";
        assert_eq!(memory_value(input, "MemTotal"), 456 * 1024);
        assert_eq!(memory_value(input, "MemAvailable"), 123 * 1024);
        assert_eq!(memory_value(input, "Missing"), 0);
    }
    #[test]
    fn malformed_or_missing_counters_do_not_invent_capacity() {
        assert_eq!(memory_value("MemTotal: invalid kB", "MemTotal"), 0);
        assert_eq!(memory_value("", "MemTotal"), 0);
    }
}
