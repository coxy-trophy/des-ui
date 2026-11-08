//! Removable drives for the file manager: what is plugged in, and mounting and
//! ejecting it. The list comes from `lsblk`, read in the background every few
//! seconds while something asks; mounting and ejecting go through `udisksctl`,
//! which asks polkit, so nothing here needs to be root.

use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A file system on a drive that can be unplugged.
#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    /// The block device of the file system, such as /dev/sdb1.
    pub device: String,
    /// The whole drive it is on, which is powered off on eject.
    pub disk: String,
    pub name: String,
    pub size: u64,
    pub mountpoint: Option<PathBuf>,
}

/// Reads `lsblk -J -b -o PATH,LABEL,SIZE,MOUNTPOINT,HOTPLUG,RM,TYPE,FSTYPE,MODEL`.
pub fn parse(json: &str) -> Vec<Drive> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let mut drives = Vec::new();
    for device in value["blockdevices"].as_array().into_iter().flatten() {
        walk(device, None, &mut drives);
    }
    drives
}

fn walk(node: &serde_json::Value, disk: Option<(&str, &str)>, out: &mut Vec<Drive>) {
    let path = node["path"].as_str().unwrap_or_default();
    let kind = node["type"].as_str().unwrap_or_default();
    let flag = |name: &str| node[name].as_bool().unwrap_or(false) || node[name].as_str() == Some("1");
    let model = node["model"].as_str().map(str::trim).filter(|m| !m.is_empty());
    let (disk_path, disk_model) = match (kind, disk) {
        ("disk", _) => (path, model.unwrap_or("")),
        (_, Some(parent)) => parent,
        _ => (path, model.unwrap_or("")),
    };
    let fstype = node["fstype"].as_str().unwrap_or_default();
    let mountpoint = node["mountpoint"].as_str().filter(|m| !m.is_empty()).map(PathBuf::from);
    let system = mountpoint.as_ref().is_some_and(|m| m == std::path::Path::new("/") || m.starts_with("/boot") || m.to_string_lossy() == "[SWAP]");
    if (flag("hotplug") || flag("rm")) && matches!(kind, "part" | "disk") && !fstype.is_empty() && fstype != "swap" && !system {
        let size = node["size"].as_u64().or_else(|| node["size"].as_str().and_then(|s| s.parse().ok())).unwrap_or(0);
        let label = node["label"].as_str().map(str::trim).filter(|l| !l.is_empty());
        let name = match (label, disk_model) {
            (Some(label), _) => label.to_string(),
            (None, model) if !model.is_empty() => model.to_string(),
            _ => format!("{} drive", human(size)),
        };
        out.push(Drive { device: path.to_string(), disk: disk_path.to_string(), name, size, mountpoint });
    }
    for child in node["children"].as_array().into_iter().flatten() {
        walk(child, Some((disk_path, disk_model)), out);
    }
}

fn human(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 10.0 { format!("{gb:.0} GB") } else if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

struct Cache { read: Option<Instant>, drives: Vec<Drive>, reading: bool }
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache { read: None, drives: Vec::new(), reading: false }))
}

/// The drives as last read, starting a fresh read when that is over three seconds old.
pub fn snapshot(ctx: &egui::Context) -> Vec<Drive> {
    let mut state = cache().lock().unwrap();
    let stale = state.read.is_none_or(|at| at.elapsed() > Duration::from_secs(3));
    if stale && !state.reading {
        state.reading = true;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let output = std::process::Command::new("lsblk")
                .args(["-J", "-b", "-o", "PATH,LABEL,SIZE,MOUNTPOINT,HOTPLUG,RM,TYPE,FSTYPE,MODEL"])
                .stderr(std::process::Stdio::null()).output();
            let drives = output.ok().filter(|o| o.status.success()).map(|o| parse(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default();
            let mut cache = cache().lock().unwrap();
            let changed = cache.drives != drives;
            cache.drives = drives; cache.read = Some(Instant::now()); cache.reading = false;
            if changed { ctx.request_repaint(); }
        });
    }
    state.drives.clone()
}

fn forget() { cache().lock().unwrap().read = None; }

fn udisks(args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("udisksctl").args(args).output().map_err(|e| format!("udisksctl could not run: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() { Ok(text) } else {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(error.lines().last().unwrap_or("udisksctl failed").trim_start_matches("Error ").to_string())
    }
}

/// "Mounted /dev/sdb1 at /media/you/STICK" → the folder.
pub fn mounted_at(output: &str) -> Option<PathBuf> {
    output.split(" at ").nth(1).map(|rest| PathBuf::from(rest.trim().trim_end_matches('.')))
}

/// Mounts `device` in the background; the answer is where it can be opened.
pub fn mount(device: String, ctx: &egui::Context) -> Receiver<Result<PathBuf, String>> {
    let (send, receive) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = udisks(&["mount", "-b", &device]).and_then(|out| mounted_at(&out).ok_or_else(|| format!("Mounted, but udisksctl did not say where: {out}")));
        forget(); let _ = send.send(result); ctx.request_repaint();
    });
    receive
}

/// Unmounts the drive's file system and powers the drive off, so it can be pulled.
pub fn eject(drive: Drive, ctx: &egui::Context) -> Receiver<Result<(), String>> {
    let (send, receive) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = if drive.mountpoint.is_some() { udisks(&["unmount", "-b", &drive.device]).map(|_| ()) } else { Ok(()) };
        if result.is_ok() {
            // Powering off fails while another of its partitions is still mounted;
            // the unmount alone already makes this one safe to pull.
            let _ = udisks(&["power-off", "-b", &drive.disk]);
        }
        forget(); let _ = send.send(result); ctx.request_repaint();
    });
    receive
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removable_file_systems_are_listed_and_system_disks_are_not() {
        let json = r#"{"blockdevices":[
          {"path":"/dev/nvme0n1","label":null,"size":512110190592,"mountpoint":null,"hotplug":false,"rm":false,"type":"disk","fstype":null,"model":"Internal SSD",
           "children":[{"path":"/dev/nvme0n1p2","label":null,"size":500000000000,"mountpoint":"/","hotplug":false,"rm":false,"type":"part","fstype":"ext4","model":null}]},
          {"path":"/dev/sdb","label":null,"size":32010928128,"mountpoint":null,"hotplug":true,"rm":true,"type":"disk","fstype":null,"model":"Kingston DataTraveler",
           "children":[{"path":"/dev/sdb1","label":"STICK","size":32000000000,"mountpoint":"/media/you/STICK","hotplug":true,"rm":true,"type":"part","fstype":"vfat","model":null},
                       {"path":"/dev/sdb2","label":"","size":8000000,"mountpoint":null,"hotplug":true,"rm":true,"type":"part","fstype":"ext4","model":null}]},
          {"path":"/dev/loop0","label":null,"size":77574144,"mountpoint":"/snap/core22/1","hotplug":false,"rm":false,"type":"loop","fstype":"squashfs","model":null}]}"#;
        let drives = parse(json);
        assert_eq!(drives.len(), 2);
        assert_eq!(drives[0], Drive { device: "/dev/sdb1".into(), disk: "/dev/sdb".into(), name: "STICK".into(), size: 32000000000, mountpoint: Some("/media/you/STICK".into()) });
        assert_eq!(drives[1].name, "Kingston DataTraveler");
        assert_eq!(drives[1].mountpoint, None);
        assert!(parse("not json").is_empty());
    }
    #[test]
    fn the_mount_point_is_read_from_udisksctl() {
        assert_eq!(mounted_at("Mounted /dev/sdb1 at /media/you/MY STICK"), Some(PathBuf::from("/media/you/MY STICK")));
        assert_eq!(mounted_at("Mounted /dev/sdb1 at /run/media/you/STICK."), Some(PathBuf::from("/run/media/you/STICK")));
        assert_eq!(mounted_at("nothing"), None);
    }
}
