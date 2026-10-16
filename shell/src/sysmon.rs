//! Samples CPU, memory, process and network statistics once per second.

use std::sync::{Arc, Mutex, mpsc};
use std::sync::atomic::{AtomicU8, Ordering};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use sysinfo::{Networks, ProcessesToUpdate, System};

pub const HISTORY: usize = 60;
const TOP_PROCESSES: usize = 6;

#[derive(Clone)]
pub struct ProcInfo {
    pub name: String,
    pub cpu: f32,
    pub mem: u64,
}

pub struct SysMon {
    latest: Arc<Mutex<Option<Snapshot>>>,
    // Disconnecting this channel wakes the worker and ends sampling.
    _stop: mpsc::Sender<()>,
    pub host: String,
    pub os: String,
    pub kernel: String,
    /// Overall CPU usage in percent, oldest first.
    pub cpu_hist: VecDeque<f32>,
    pub cores: Vec<f32>,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub top: Vec<ProcInfo>,
    pub iface: String,
    pub ip: String,
    /// Download and upload rates in bytes per second, oldest first.
    pub rx_hist: VecDeque<f32>,
    pub tx_hist: VecDeque<f32>,
    pub rx_total: u64,
    pub tx_total: u64,
    pub battery: Option<u8>,
    /// The battery in full: whether it is plugged in, and the time left.
    pub power: Option<Battery>,
    /// Screen backlight, in percent of its range.
    pub brightness: Option<u8>,
    /// Wireless link quality, in percent.
    pub wifi: Option<u8>,
    /// Output volume in percent, sampled in the background.
    volume: Arc<AtomicU8>,
}

/// Stands for "not known" in the shared volume reading.
const NO_VOLUME: u8 = u8::MAX;

impl SysMon {
    pub fn new() -> SysMon {
        let mut sampler = None;
        let (latest, stop) = watch_samples(move || {
            sampler.get_or_insert_with(Sampler::new).sample()
        });
        SysMon {
            latest,
            _stop: stop,
            host: System::host_name().unwrap_or_else(|| "localhost".into()),
            os: System::name().unwrap_or_else(|| "Linux".into()),
            kernel: System::kernel_version().unwrap_or_default(),
            cpu_hist: VecDeque::from(vec![0.0; HISTORY]),
            cores: Vec::new(),
            mem_used: 0,
            mem_total: 0,
            swap_used: 0,
            swap_total: 0,
            top: Vec::new(),
            iface: String::new(),
            ip: String::new(),
            rx_hist: VecDeque::from(vec![0.0; HISTORY]),
            tx_hist: VecDeque::from(vec![0.0; HISTORY]),
            rx_total: 0,
            tx_total: 0,
            battery: None,
            power: None,
            brightness: None,
            wifi: None,
            volume: watch_volume(),
        }
    }

    /// Applies the latest complete sample without waiting for the worker.
    pub fn tick(&mut self) {
        let snapshot = self.latest.try_lock().ok().and_then(|mut latest| latest.take());
        let Some(snapshot) = snapshot else { return; };
        self.cpu_hist = snapshot.cpu_hist;
        self.cores = snapshot.cores;
        self.mem_used = snapshot.mem_used;
        self.mem_total = snapshot.mem_total;
        self.swap_used = snapshot.swap_used;
        self.swap_total = snapshot.swap_total;
        self.top = snapshot.top;
        self.iface = snapshot.iface;
        self.ip = snapshot.ip;
        self.rx_hist = snapshot.rx_hist;
        self.tx_hist = snapshot.tx_hist;
        self.rx_total = snapshot.rx_total;
        self.tx_total = snapshot.tx_total;
        self.power = snapshot.power;
        self.battery = snapshot.battery;
        self.brightness = snapshot.brightness;
        self.wifi = snapshot.wifi;
    }

    pub fn volume(&self) -> Option<u8> {
        match self.volume.load(Ordering::Relaxed) {
            NO_VOLUME => None,
            volume => Some(volume),
        }
    }

    pub fn uptime() -> u64 {
        System::uptime()
    }
}

#[derive(Clone)]
struct Snapshot {
    cpu_hist: VecDeque<f32>,
    cores: Vec<f32>,
    mem_used: u64,
    mem_total: u64,
    swap_used: u64,
    swap_total: u64,
    top: Vec<ProcInfo>,
    iface: String,
    ip: String,
    rx_hist: VecDeque<f32>,
    tx_hist: VecDeque<f32>,
    rx_total: u64,
    tx_total: u64,
    battery: Option<u8>,
    power: Option<Battery>,
    brightness: Option<u8>,
    wifi: Option<u8>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            cpu_hist: VecDeque::from(vec![0.0; HISTORY]),
            cores: Vec::new(),
            mem_used: 0,
            mem_total: 0,
            swap_used: 0,
            swap_total: 0,
            top: Vec::new(),
            iface: String::new(),
            ip: String::new(),
            rx_hist: VecDeque::from(vec![0.0; HISTORY]),
            tx_hist: VecDeque::from(vec![0.0; HISTORY]),
            rx_total: 0,
            tx_total: 0,
            battery: None,
            power: None,
            brightness: None,
            wifi: None,
        }
    }
}

struct Sampler {
    sys: System,
    nets: Networks,
    last: Option<Instant>,
    snapshot: Snapshot,
}

impl Sampler {
    fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        let nets = Networks::new_with_refreshed_list();
        Self { sys, nets, last: None, snapshot: Snapshot::default() }
    }

    fn sample(&mut self) -> Snapshot {
        // Network deltas span the actual refresh interval, independent of UI ticks.
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        self.nets.refresh(true);
        let now = Instant::now();
        let elapsed = self.last.replace(now).map(|last| now.duration_since(last));

        push(&mut self.snapshot.cpu_hist, self.sys.global_cpu_usage());
        self.snapshot.cores = self.sys.cpus().iter().map(|c| c.cpu_usage()).collect();
        self.snapshot.mem_used = self.sys.used_memory();
        self.snapshot.mem_total = self.sys.total_memory();
        self.snapshot.swap_used = self.sys.used_swap();
        self.snapshot.swap_total = self.sys.total_swap();

        let ncores = self.snapshot.cores.len().max(1) as f32;
        let mut procs: Vec<ProcInfo> = self
            .sys
            .processes()
            .values()
            // Threads show up as processes on Linux; only keep thread-group leaders.
            .filter(|p| p.thread_kind().is_none())
            .map(|p| ProcInfo {
                name: p.name().to_string_lossy().into_owned(),
                cpu: p.cpu_usage() / ncores,
                mem: p.memory(),
            })
            .collect();
        procs.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.mem.cmp(&a.mem)));
        procs.truncate(TOP_PROCESSES);
        self.snapshot.top = procs;

        // Report on the busiest real interface.
        let active = self
            .nets
            .list()
            .iter()
            .filter(|(name, _)| name.as_str() != "lo")
            .max_by_key(|(_, data)| data.total_received() + data.total_transmitted());
        match active {
            Some((name, data)) => {
                // A newly selected interface has no displayed rate baseline yet.
                let elapsed = elapsed.filter(|_| self.snapshot.iface == *name);
                self.snapshot.iface = name.clone();
                self.snapshot.ip = data
                    .ip_networks()
                    .iter()
                    .find(|n| n.addr.is_ipv4())
                    .map(|n| n.addr.to_string())
                    .unwrap_or_default();
                push(&mut self.snapshot.rx_hist, network_rate(data.received(), elapsed));
                push(&mut self.snapshot.tx_hist, network_rate(data.transmitted(), elapsed));
                self.snapshot.rx_total = data.total_received();
                self.snapshot.tx_total = data.total_transmitted();
            }
            None => {
                self.snapshot.iface.clear();
                self.snapshot.ip.clear();
                self.snapshot.rx_total = 0;
                self.snapshot.tx_total = 0;
                push(&mut self.snapshot.rx_hist, 0.0);
                push(&mut self.snapshot.tx_hist, 0.0);
            }
        }

        self.snapshot.power = read_battery();
        self.snapshot.battery = self.snapshot.power.map(|battery| battery.percent);
        self.snapshot.brightness = read_brightness();
        self.snapshot.wifi = read_wifi();
        self.snapshot.clone()
    }
}

/// One slot bounds memory even while the UI is hidden or stalled. Sampling and
/// publication never hold the mutex at the same time as expensive host reads.
fn watch_samples(mut sample: impl FnMut() -> Snapshot + Send + 'static)
    -> (Arc<Mutex<Option<Snapshot>>>, mpsc::Sender<()>)
{
    let latest = Arc::new(Mutex::new(None));
    let writer = latest.clone();
    let (stop, stopped) = mpsc::channel();
    std::thread::spawn(move || {
        loop {
            if matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
                break;
            }
            let snapshot = sample();
            if matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
                break;
            }
            if let Ok(mut latest) = writer.lock() {
                *latest = Some(snapshot);
            }
            if stopped.recv_timeout(Duration::from_secs(1)) != Err(mpsc::RecvTimeoutError::Timeout) {
                break;
            }
        }
    });
    (latest, stop)
}

fn network_rate(bytes: u64, elapsed: Option<Duration>) -> f32 {
    elapsed.filter(|duration| !duration.is_zero())
        .map(|duration| bytes as f32 / duration.as_secs_f32())
        .unwrap_or(0.0)
}

fn push(hist: &mut VecDeque<f32>, value: f32) {
    hist.push_back(value);
    while hist.len() > HISTORY {
        hist.pop_front();
    }
}

/// Asks the audio server for the output volume every two seconds, on a thread of its
/// own because that means running a program.
fn watch_volume() -> Arc<AtomicU8> {
    let shared = Arc::new(AtomicU8::new(NO_VOLUME));
    let writer = shared.clone();
    std::thread::spawn(move || {
        // Stops once the monitor that reads this is gone.
        while Arc::strong_count(&writer) > 1 {
            let reading = std::process::Command::new("wpctl")
                .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
                .output()
                .ok()
                .and_then(|output| parse_volume(&String::from_utf8_lossy(&output.stdout)));
            writer.store(reading.unwrap_or(NO_VOLUME), Ordering::Relaxed);
            std::thread::sleep(Duration::from_secs(2));
        }
    });
    shared
}

/// Reads `wpctl get-volume` output such as `Volume: 0.55` or `Volume: 0.40 [MUTED]`.
fn parse_volume(output: &str) -> Option<u8> {
    if output.contains("MUTED") {
        return Some(0);
    }
    let level: f32 = output.split_whitespace().nth(1)?.parse().ok()?;
    Some((level * 100.0).round().clamp(0.0, 200.0) as u8)
}

fn read_brightness() -> Option<u8> {
    let device = std::fs::read_dir("/sys/class/backlight").ok()?.flatten().next()?.path();
    let read = |file: &str| -> Option<f32> {
        std::fs::read_to_string(device.join(file)).ok()?.trim().parse().ok()
    };
    let (current, max) = (read("brightness")?, read("max_brightness")?);
    (max > 0.0).then(|| (current / max * 100.0).round() as u8)
}

/// Link quality of the first wireless interface, from the kernel's table (out of 70).
fn read_wifi() -> Option<u8> {
    let table = std::fs::read_to_string("/proc/net/wireless").ok()?;
    let line = table.lines().nth(2)?;
    let quality: f32 = line.split_whitespace().nth(2)?.trim_end_matches('.').parse().ok()?;
    Some((quality / 70.0 * 100.0).round().clamp(0.0, 100.0) as u8)
}

/// How the battery stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    pub percent: u8,
    /// On mains power, whether still filling or full.
    pub plugged: bool,
    /// Minutes until empty, or until full when plugged in, where the battery says.
    pub minutes: Option<u32>,
}

/// Works out the time left from what a battery reports: how much it holds (`now`),
/// how much when full, and how fast that is changing, all in the same unit.
pub fn minutes_left(now: f64, full: f64, rate: f64, plugged: bool) -> Option<u32> {
    if rate <= 0.0 {
        return None;
    }
    let hours = if plugged { (full - now).max(0.0) / rate } else { now / rate };
    let minutes = (hours * 60.0).round();
    // A reading of days is a battery that is barely being drawn on, not a forecast.
    (minutes > 0.0 && minutes < 48.0 * 60.0).then_some(minutes as u32)
}

fn read_battery() -> Option<Battery> {
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let path = entry.path();
        let text = |file: &str| std::fs::read_to_string(path.join(file)).ok().map(|text| text.trim().to_string());
        let number = |file: &str| text(file).and_then(|text| text.parse::<f64>().ok());
        if text("type").as_deref() != Some("Battery") {
            continue;
        }
        let Some(percent) = text("capacity").and_then(|text| text.parse().ok()) else {
            continue;
        };
        let status = text("status").unwrap_or_default();
        let plugged = status != "Discharging";
        // Batteries count in charge or in energy, never both.
        let reading = match (number("charge_now"), number("charge_full"), number("current_now")) {
            (Some(now), Some(full), Some(rate)) => Some((now, full, rate)),
            _ => match (number("energy_now"), number("energy_full"), number("power_now")) {
                (Some(now), Some(full), Some(rate)) => Some((now, full, rate)),
                _ => None,
            },
        };
        let minutes = reading
            .filter(|_| status != "Full" && status != "Not charging")
            .and_then(|(now, full, rate)| minutes_left(now, full, rate.abs(), plugged));
        return Some(Battery { percent, plugged, minutes });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn works_out_the_time_left() {
        // Half of a 4 Ah battery, drawn at 1 A: two hours to empty.
        assert_eq!(minutes_left(2.0, 4.0, 1.0, false), Some(120));
        // The same, filling at 2 A: an hour to full.
        assert_eq!(minutes_left(2.0, 4.0, 2.0, true), Some(60));
        assert_eq!(minutes_left(2.0, 4.0, 0.0, false), None, "no rate, no forecast");
        assert_eq!(minutes_left(4.0, 4.0, 0.001, false), None, "days are not a forecast");
    }

    #[test]
    fn reads_volume() {
        assert_eq!(parse_volume("Volume: 0.55\n"), Some(55));
        assert_eq!(parse_volume("Volume: 0.40 [MUTED]\n"), Some(0));
        assert_eq!(parse_volume("Volume: 1.25"), Some(125));
        assert_eq!(parse_volume(""), None);
    }

    #[test]
    fn tick_does_not_take_a_sample_on_the_calling_thread() {
        let mut mon = SysMon::new();
        let latest = mon.latest.clone();
        let publishing = latest.lock().unwrap();
        let (completed, done) = mpsc::channel();
        let ui = std::thread::spawn(move || {
            mon.tick();
            completed.send(mon.mem_total).unwrap();
        });
        let result = done.recv_timeout(Duration::from_secs(1));
        // Release even on failure so a blocking-lock regression cannot hang tests.
        drop(publishing);
        ui.join().unwrap();
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn stalled_sampler_does_not_block_ticks_and_stops_after_drop() {
        struct NotifyDrop(mpsc::Sender<()>);
        impl Drop for NotifyDrop {
            fn drop(&mut self) { let _ = self.0.send(()); }
        }
        let (entered, started) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let (exited, stopped) = mpsc::channel();
        let guard = NotifyDrop(exited);
        let (latest, stop) = watch_samples(move || {
            let _keep_until_worker_exits = &guard;
            entered.send(()).unwrap();
            released.recv().unwrap();
            Snapshot::default()
        });
        let mut mon = SysMon::new();
        mon.latest = latest;
        mon._stop = stop;
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let start = Instant::now();
        for _ in 0..100 { mon.tick(); }
        assert!(start.elapsed() < Duration::from_millis(100));
        drop(mon);
        release.send(()).unwrap();
        stopped.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(started.try_recv().is_err(), "sampling continued after monitor drop");
    }

    #[test]
    fn latest_snapshot_preserves_history_when_ui_skips_samples() {
        let mut sample = Snapshot::default();
        let mut value = 0;
        let (latest, stop) = watch_samples(move || {
            value += 1;
            push(&mut sample.cpu_hist, value as f32);
            sample.mem_total = value;
            sample.clone()
        });
        let mut mon = SysMon::new();
        mon.latest = latest.clone();
        mon._stop = stop;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let ready = latest.lock().unwrap().as_ref().is_some_and(|sample| sample.mem_total >= 2);
            if ready { break; }
            assert!(Instant::now() < deadline, "second background sample did not arrive");
            std::thread::yield_now();
        }
        mon.tick();
        assert!(mon.mem_total >= 2);
        assert_eq!(mon.cpu_hist.len(), HISTORY);
        let values: Vec<_> = mon.cpu_hist.iter().rev().take(2).copied().collect();
        assert_eq!(values, vec![mon.mem_total as f32, (mon.mem_total - 1) as f32]);
        assert!(latest.lock().unwrap().is_none(), "tick must consume the one pending snapshot");
    }

    #[test]
    fn network_rates_use_sampling_interval_and_require_a_baseline() {
        assert_eq!(network_rate(600, Some(Duration::from_secs(2))), 300.0);
        assert_eq!(network_rate(600, Some(Duration::from_millis(500))), 1200.0);
        assert_eq!(network_rate(600, None), 0.0);
        assert_eq!(network_rate(600, Some(Duration::ZERO)), 0.0);
    }

    #[test]
    fn samples_have_sane_values() {
        let mut mon = SysMon::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while mon.mem_total == 0 && Instant::now() < deadline {
            mon.tick();
            std::thread::yield_now();
        }
        assert!(mon.mem_total > 0, "background sample did not arrive before deadline");
        assert!(mon.mem_used <= mon.mem_total);
        assert!(!mon.cores.is_empty());
        assert!(!mon.top.is_empty());
        assert_eq!(mon.cpu_hist.len(), HISTORY);
    }
}
