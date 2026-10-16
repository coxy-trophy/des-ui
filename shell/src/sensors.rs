//! Readings sysinfo does not give: temperatures and fans from the kernel's hwmon
//! devices, how busy the GPU is, disk traffic, the load average, how worn the battery
//! is, and which systemd units have failed. Parsing takes text, so tests use
//! fixtures; reading happens on the sampler's thread, bounded in what it opens and
//! how long it waits for a program.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How many hwmon devices, and inputs per device, are looked at.
const MAX_DEVICES: usize = 16;
const MAX_INPUTS: usize = 32;
/// Failed units past this are counted, not named.
pub const MAX_FAILED_UNITS: usize = 16;
/// How much of a program's output is kept.
const MAX_OUTPUT: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Temperature {
    /// What is measured, as a word for the panel: CPU, GPU, NVME, BOARD…
    pub label: String,
    pub degrees: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fan {
    pub label: String,
    pub rpm: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gpu {
    pub name: String,
    /// Percent of the time it was busy.
    pub busy: u8,
}

/// One hwmon device as read: its driver's name and its numbered inputs.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct HwmonDevice {
    pub name: String,
    /// Each input's label, where the driver gives one, and its millidegrees.
    pub temps: Vec<(Option<String>, i64)>,
    pub fans: Vec<(Option<String>, u32)>,
}

/// A mounted filesystem and how full it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Filesystem {
    pub mount: String,
    /// Used space as a fraction of the total.
    pub used: f32,
}

/// What a driver measures, in a word.
pub fn device_label(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let word = if ["coretemp", "k10temp", "zenpower", "cpu_thermal", "cpu"].iter().any(|p| lower.starts_with(p)) {
        "CPU"
    } else if ["amdgpu", "nouveau", "i915", "xe", "radeon"].iter().any(|p| lower.starts_with(p)) {
        "GPU"
    } else if lower.starts_with("nvme") {
        "NVME"
    } else if lower.starts_with("drivetemp") {
        "DISK"
    } else if lower.starts_with("acpitz") || lower.starts_with("thinkpad") || lower.starts_with("pch") {
        "BOARD"
    } else if lower.contains("wifi") || lower.starts_with("mt76") || lower.starts_with("ath") {
        "WIFI"
    } else if lower.starts_with("bat") {
        "BATTERY"
    } else {
        return name.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect::<String>().to_uppercase();
    };
    word.to_string()
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|text| text.trim().to_string())
}

/// Reads every hwmon device under `root` (`/sys/class/hwmon` on a real machine).
pub fn read_hwmon_at(root: &Path) -> Vec<HwmonDevice> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut devices = Vec::new();
    for entry in entries.flatten().take(MAX_DEVICES) {
        let dir = entry.path();
        let Some(name) = read_trimmed(&dir.join("name")) else { continue };
        let mut device = HwmonDevice { name, ..Default::default() };
        for n in 1..=MAX_INPUTS {
            let temp = read_trimmed(&dir.join(format!("temp{n}_input"))).and_then(|t| t.parse::<i64>().ok());
            if let Some(millis) = temp {
                let label = read_trimmed(&dir.join(format!("temp{n}_label")));
                device.temps.push((label, millis));
            }
            let fan = read_trimmed(&dir.join(format!("fan{n}_input"))).and_then(|t| t.parse::<u32>().ok());
            if let Some(rpm) = fan {
                let label = read_trimmed(&dir.join(format!("fan{n}_label")));
                device.fans.push((label, rpm));
            }
        }
        if !device.temps.is_empty() || !device.fans.is_empty() {
            devices.push(device);
        }
    }
    devices
}

pub fn read_hwmon() -> Vec<HwmonDevice> {
    read_hwmon_at(Path::new("/sys/class/hwmon"))
}

/// One temperature per kind of thing, the hottest reading of each, hottest first.
/// Readings outside what a sensor can mean are dropped.
pub fn temperatures(devices: &[HwmonDevice]) -> Vec<Temperature> {
    let mut out: Vec<Temperature> = Vec::new();
    for device in devices {
        let label = device_label(&device.name);
        let hottest = device
            .temps
            .iter()
            .map(|(_, millis)| *millis as f32 / 1000.0)
            .filter(|degrees| (-40.0..=200.0).contains(degrees))
            .fold(None, |best: Option<f32>, d| Some(best.map_or(d, |b| b.max(d))));
        let Some(degrees) = hottest else { continue };
        match out.iter_mut().find(|t| t.label == label) {
            Some(known) => known.degrees = known.degrees.max(degrees),
            None => out.push(Temperature { label, degrees }),
        }
    }
    out.sort_by(|a, b| b.degrees.total_cmp(&a.degrees));
    out
}

/// Every fan, fastest first, labelled by the driver's name or its own label.
pub fn fans(devices: &[HwmonDevice]) -> Vec<Fan> {
    let mut out: Vec<Fan> = devices
        .iter()
        .flat_map(|device| {
            device.fans.iter().map(move |(label, rpm)| Fan {
                label: label.clone().map(|l| l.to_uppercase()).unwrap_or_else(|| device_label(&device.name)),
                rpm: *rpm,
            })
        })
        .collect();
    out.sort_by(|a, b| b.rpm.cmp(&a.rpm));
    out
}

/// A GPU whose driver reports how busy it is under `/sys/class/drm` (amdgpu does).
pub fn read_drm_gpu_at(root: &Path) -> Option<Gpu> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten().take(16) {
        let device = entry.path().join("device");
        let Some(busy) = read_trimmed(&device.join("gpu_busy_percent")) else { continue };
        let Ok(busy) = busy.parse::<u8>() else { continue };
        let name = match read_trimmed(&device.join("vendor")).as_deref() {
            Some("0x1002") => "AMD",
            Some("0x8086") => "INTEL",
            Some("0x10de") => "NVIDIA",
            _ => "GPU",
        };
        return Some(Gpu { name: name.to_string(), busy: busy.min(100) });
    }
    None
}

/// `nvidia-smi --query-gpu=name,utilization.gpu --format=csv,noheader,nounits`
/// prints `NVIDIA GeForce RTX 3060, 12`.
pub fn parse_nvidia_smi(text: &str) -> Option<Gpu> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let (name, busy) = line.rsplit_once(',')?;
    let busy: u8 = busy.trim().parse().ok()?;
    let name = name.trim().trim_start_matches("NVIDIA ").trim();
    Some(Gpu { name: if name.is_empty() { "NVIDIA".into() } else { name.to_string() }, busy: busy.min(100) })
}

/// Where a program is, if it is on the PATH at all. Looked up once by the sampler.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|candidate| candidate.is_file())
}

/// Runs a program and returns what it printed, or nothing if it took longer than
/// `timeout`, in which case it is killed. Output is read on its own thread and capped
/// so a talkative program can stall nothing and fill nothing.
pub fn run_bounded(program: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.by_ref().take(MAX_OUTPUT).read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let bytes = reader.join().ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Whole disks in `/proc/diskstats`, with bytes read and written since boot.
/// Partitions, loop devices and mapped devices would count the same traffic twice.
pub fn parse_diskstats(text: &str) -> Vec<(String, u64, u64)> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 || !is_whole_disk(fields[2]) {
                return None;
            }
            let read: u64 = fields[5].parse().ok()?;
            let written: u64 = fields[9].parse().ok()?;
            Some((fields[2].to_string(), read * 512, written * 512))
        })
        .take(32)
        .collect()
}

fn is_whole_disk(name: &str) -> bool {
    let ends_in_digit = name.ends_with(|c: char| c.is_ascii_digit());
    if let Some(rest) = name.strip_prefix("nvme").or_else(|| name.strip_prefix("mmcblk")) {
        // nvme0n1 is a disk, nvme0n1p1 a partition; mmcblk0 a card, mmcblk0p1 a partition.
        return !rest.contains('p') && ends_in_digit;
    }
    ["sd", "vd", "hd", "xvd"].iter().any(|p| name.starts_with(p)) && !ends_in_digit
}

/// Bytes per second across all disks between two readings.
pub fn disk_rates(before: &[(String, u64, u64)], after: &[(String, u64, u64)], elapsed: Duration) -> (f32, f32) {
    if elapsed.is_zero() {
        return (0.0, 0.0);
    }
    let (mut read, mut written) = (0u64, 0u64);
    for (name, r, w) in after {
        if let Some((_, r0, w0)) = before.iter().find(|(n, _, _)| n == name) {
            read += r.saturating_sub(*r0);
            written += w.saturating_sub(*w0);
        }
    }
    let seconds = elapsed.as_secs_f32();
    (read as f32 / seconds, written as f32 / seconds)
}

pub fn read_diskstats() -> Vec<(String, u64, u64)> {
    std::fs::read_to_string("/proc/diskstats").map(|t| parse_diskstats(&t)).unwrap_or_default()
}

/// The three load averages in `/proc/loadavg`.
pub fn parse_loadavg(text: &str) -> Option<[f32; 3]> {
    let mut words = text.split_whitespace();
    let one = words.next()?.parse().ok()?;
    let five = words.next()?.parse().ok()?;
    let fifteen = words.next()?.parse().ok()?;
    Some([one, five, fifteen])
}

pub fn read_loadavg() -> Option<[f32; 3]> {
    std::fs::read_to_string("/proc/loadavg").ok().and_then(|t| parse_loadavg(&t))
}

/// How much a battery still holds when full, as a percent of what it was built to.
pub fn battery_health(full: f64, design: f64) -> Option<u8> {
    (design > 0.0 && full > 0.0).then(|| (full / design * 100.0).round().clamp(0.0, 100.0) as u8)
}

pub fn read_battery_health_at(root: &Path) -> Option<u8> {
    for entry in std::fs::read_dir(root).ok()?.flatten().take(8) {
        let dir = entry.path();
        if read_trimmed(&dir.join("type")).as_deref() != Some("Battery") {
            continue;
        }
        let number = |file: &str| read_trimmed(&dir.join(file)).and_then(|t| t.parse::<f64>().ok());
        let pair = number("energy_full")
            .zip(number("energy_full_design"))
            .or_else(|| number("charge_full").zip(number("charge_full_design")));
        if let Some((full, design)) = pair {
            return battery_health(full, design);
        }
    }
    None
}

pub fn read_battery_health() -> Option<u8> {
    read_battery_health_at(Path::new("/sys/class/power_supply"))
}

/// Unit names from `systemctl --failed --no-legend --plain`, whose lines read
/// `foo.service loaded failed failed The description`, sometimes with a mark first.
pub fn parse_failed_units(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start_matches(['●', '*', '×', ' ']);
            let name = line.split_whitespace().next()?;
            name.contains('.').then(|| name.to_string())
        })
        .take(MAX_FAILED_UNITS)
        .collect()
}

pub fn read_failed_units(systemctl: &Path) -> Option<Vec<String>> {
    run_bounded(systemctl, &["--failed", "--no-legend", "--plain"], Duration::from_secs(2)).map(|t| parse_failed_units(&t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, file: &str, text: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(file), text).unwrap();
    }

    #[test]
    fn reads_hwmon_devices_and_picks_the_hottest_of_each_kind() {
        let root = tempfile::tempdir().unwrap();
        let core = root.path().join("hwmon6");
        write(&core, "name", "coretemp\n");
        write(&core, "temp1_input", "61000\n");
        write(&core, "temp1_label", "Package id 0\n");
        write(&core, "temp2_input", "58000\n");
        write(&core, "temp3_input", "-274000\n"); // nonsense from a broken sensor
        let board = root.path().join("hwmon0");
        write(&board, "name", "acpitz\n");
        write(&board, "temp1_input", "42000\n");
        let board2 = root.path().join("hwmon1");
        write(&board2, "name", "acpitz\n");
        write(&board2, "temp1_input", "45500\n");
        write(&board2, "fan1_input", "1200\n");
        write(&board2, "fan1_label", "cpu fan\n");
        let empty = root.path().join("hwmon2");
        write(&empty, "name", "BAT1\n");

        let devices = read_hwmon_at(root.path());
        assert_eq!(devices.len(), 3, "a device with no inputs is not listed");
        let temps = temperatures(&devices);
        assert_eq!(temps.len(), 2);
        assert_eq!(temps[0], Temperature { label: "CPU".into(), degrees: 61.0 });
        assert_eq!(temps[1], Temperature { label: "BOARD".into(), degrees: 45.5 });
        assert_eq!(fans(&devices), vec![Fan { label: "CPU FAN".into(), rpm: 1200 }]);
        assert!(read_hwmon_at(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn labels_drivers_by_what_they_measure() {
        assert_eq!(device_label("k10temp"), "CPU");
        assert_eq!(device_label("amdgpu"), "GPU");
        assert_eq!(device_label("nvme"), "NVME");
        assert_eq!(device_label("iwlwifi_1_2"), "WIFI");
        assert_eq!(device_label("acpitz_0"), "BOARD");
        assert_eq!(device_label("some weird-driver"), "SOMEWEIR");
    }

    #[test]
    fn reads_gpu_busy_from_drm_and_nvidia_smi() {
        let root = tempfile::tempdir().unwrap();
        let card = root.path().join("card1").join("device");
        write(&card, "gpu_busy_percent", "37\n");
        write(&card, "vendor", "0x1002\n");
        assert_eq!(read_drm_gpu_at(root.path()), Some(Gpu { name: "AMD".into(), busy: 37 }));
        assert_eq!(read_drm_gpu_at(Path::new("/nonexistent")), None);
        assert_eq!(parse_nvidia_smi("NVIDIA GeForce RTX 3060, 12\n"), Some(Gpu { name: "GeForce RTX 3060".into(), busy: 12 }));
        assert_eq!(parse_nvidia_smi(""), None);
        assert_eq!(parse_nvidia_smi("No devices were found"), None);
    }

    #[test]
    fn disk_traffic_counts_whole_disks_once() {
        let before = "   8       0 sda 100 0 2048 0 50 0 1024 0 0 0 0 0 0 0 0 0 0\n   8       1 sda1 100 0 2048 0 50 0 1024 0 0 0 0 0 0 0 0 0 0\n 259       0 nvme0n1 10 0 4096 0 1 0 512 0 0 0 0 0 0 0 0 0 0\n 259       1 nvme0n1p1 10 0 4096 0 1 0 512 0 0 0 0 0 0 0 0 0 0\n   7       0 loop0 1 0 8 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n 253       0 dm-0 9 0 999 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n";
        let parsed = parse_diskstats(before);
        let names: Vec<&str> = parsed.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, ["sda", "nvme0n1"]);
        assert_eq!(parsed[0].1, 2048 * 512);
        let after = before.replace("sda 100 0 2048 0 50 0 1024", "sda 100 0 4096 0 50 0 3072");
        let (read, written) = disk_rates(&parsed, &parse_diskstats(&after), Duration::from_secs(2));
        assert_eq!(read, 2048.0 * 512.0 / 2.0);
        assert_eq!(written, 2048.0 * 512.0 / 2.0);
        assert_eq!(disk_rates(&parsed, &parsed, Duration::ZERO), (0.0, 0.0));
        assert!(is_whole_disk("mmcblk0") && !is_whole_disk("mmcblk0p1") && is_whole_disk("vdb") && !is_whole_disk("vdb2"));
    }

    #[test]
    fn reads_the_load_average_and_battery_wear() {
        assert_eq!(parse_loadavg("3.12 2.32 1.95 1/1017 214079\n"), Some([3.12, 2.32, 1.95]));
        assert_eq!(parse_loadavg("garbage"), None);
        assert_eq!(battery_health(38_000_000.0, 50_000_000.0), Some(76));
        assert_eq!(battery_health(60.0, 50.0), Some(100));
        assert_eq!(battery_health(1.0, 0.0), None);
        let root = tempfile::tempdir().unwrap();
        let bat = root.path().join("BAT1");
        write(&bat, "type", "Battery\n");
        write(&bat, "charge_full", "3000000\n");
        write(&bat, "charge_full_design", "4000000\n");
        write(&root.path().join("ACAD"), "type", "Mains\n");
        assert_eq!(read_battery_health_at(root.path()), Some(75));
    }

    #[test]
    fn reads_failed_units() {
        let text = "● foo.service loaded failed failed Foo daemon\n  bar.timer loaded failed failed Bar\n";
        assert_eq!(parse_failed_units(text), vec!["foo.service", "bar.timer"]);
        assert!(parse_failed_units("").is_empty());
        let many: String = (0..40).map(|i| format!("u{i}.service loaded failed failed X\n")).collect();
        assert_eq!(parse_failed_units(&many).len(), MAX_FAILED_UNITS);
    }

    #[test]
    fn bounded_programs_are_killed_at_the_deadline() {
        let echo = find_program("echo").expect("echo on PATH");
        assert_eq!(run_bounded(&echo, &["hi"], Duration::from_secs(2)).as_deref(), Some("hi\n"));
        let sleep = find_program("sleep").expect("sleep on PATH");
        let start = Instant::now();
        assert_eq!(run_bounded(&sleep, &["5"], Duration::from_millis(150)), None);
        assert!(start.elapsed() < Duration::from_secs(2));
        let falsy = find_program("false").expect("false on PATH");
        assert_eq!(run_bounded(&falsy, &[], Duration::from_secs(1)), None, "a failing program's output is not trusted");
        assert_eq!(find_program("surely-not-a-program-here"), None);
    }
}
