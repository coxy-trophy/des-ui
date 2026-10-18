//! Samples CPU, memory, process, network, disk and sensor readings once per second on
//! a worker thread, and raises alerts when a reading crosses a line.

use crate::sensors::{self, Fan, Filesystem, Gpu, Temperature};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};
use sysinfo::{Disks, Networks, ProcessesToUpdate, System};

pub const HISTORY: usize = 60;
const TOP_PROCESSES: usize = 6;
/// Per-core history is kept for this many cores; the rest still show their load.
const CORE_HISTORIES: usize = 64;
/// How often failed units are asked for, and the GPU through nvidia-smi.
const UNITS_EVERY: Duration = Duration::from_secs(30);
const NVIDIA_EVERY: Duration = Duration::from_secs(5);
/// Filesystems shown, fullest first.
const FILESYSTEMS: usize = 6;
/// Alerts waiting to be shown; more than this and the oldest are dropped.
const PENDING_ALERTS: usize = 16;

#[derive(Debug, Clone, PartialEq)]
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
    /// Each core's history, for the first CORE_HISTORIES cores.
    pub core_hist: Vec<VecDeque<f32>>,
    /// Memory in use as a percent of the total, oldest first.
    pub mem_hist: VecDeque<f32>,
    pub load: Option<[f32; 3]>,
    pub temps: Vec<Temperature>,
    pub fans: Vec<Fan>,
    pub gpu: Option<Gpu>,
    pub gpu_hist: VecDeque<f32>,
    /// Disk traffic in bytes per second, oldest first, and the totals since boot.
    pub disk_read_hist: VecDeque<f32>,
    pub disk_write_hist: VecDeque<f32>,
    pub disk_read_total: u64,
    pub disk_write_total: u64,
    /// Mounted filesystems, fullest first.
    pub filesystems: Vec<Filesystem>,
    /// What the battery still holds when full, as a percent of when it was new.
    pub battery_health: Option<u8>,
    pub failed_units: Vec<String>,
    alarms: Alarms,
    processor_temperature_warnings: bool,
    alerts: Vec<Alert>,
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
    /// The machine being followed through ssh, when there is one; see [`SysMon::set_remote`].
    pub remote: Option<Following>,
    /// This machine's last sample, kept while another is shown so it can come back at once.
    local: Option<Snapshot>,
    local_names: (String, String, String),
    remote_uptime: Option<u64>,
}

/// The host the panels follow, as they should name it.
#[derive(Debug, Clone, PartialEq)]
pub struct Following {
    /// `user@host` as typed, or the host's own name once it has answered.
    pub name: String,
    /// Whether its readings are the ones shown; otherwise this machine's are, and
    /// `word` says why.
    pub live: bool,
    pub word: &'static str,
}

/// Stands for "not known" in the shared volume reading.
const NO_VOLUME: u8 = u8::MAX;

impl SysMon {
    pub fn new() -> SysMon {
        let mut sampler = None;
        let (latest, stop) = watch_samples(move || {
            sampler.get_or_insert_with(Sampler::new).sample()
        });
        let host = System::host_name().unwrap_or_else(|| "localhost".into());
        let os = System::name().unwrap_or_else(|| "Linux".into());
        let kernel = System::kernel_version().unwrap_or_default();
        SysMon {
            latest,
            _stop: stop,
            local_names: (host.clone(), os.clone(), kernel.clone()),
            host,
            os,
            kernel,
            cpu_hist: VecDeque::from(vec![0.0; HISTORY]),
            cores: Vec::new(),
            core_hist: Vec::new(),
            mem_hist: VecDeque::from(vec![0.0; HISTORY]),
            load: None,
            temps: Vec::new(),
            fans: Vec::new(),
            gpu: None,
            gpu_hist: VecDeque::from(vec![0.0; HISTORY]),
            disk_read_hist: VecDeque::from(vec![0.0; HISTORY]),
            disk_write_hist: VecDeque::from(vec![0.0; HISTORY]),
            disk_read_total: 0,
            disk_write_total: 0,
            filesystems: Vec::new(),
            battery_health: None,
            failed_units: Vec::new(),
            alarms: Alarms::default(),
            processor_temperature_warnings: true,
            alerts: Vec::new(),
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
            remote: None,
            local: None,
            remote_uptime: None,
        }
    }

    /// Suppress only CPU heat alerts, including any not yet posted to the center.
    pub fn set_processor_temperature_warnings(&mut self, enabled: bool) {
        if self.processor_temperature_warnings != enabled {
            self.processor_temperature_warnings = enabled;
            self.alarms.temperature = Latch::default();
        }
        if !enabled { self.alerts.retain(|alert| alert.kind != AlertKind::ProcessorTemperature); }
    }

    /// Applies the latest complete sample without waiting for the worker.
    pub fn tick(&mut self) {
        let snapshot = self.latest.try_lock().ok().and_then(|mut latest| latest.take());
        let Some(snapshot) = snapshot else { return; };
        // Alarms are about this machine, whatever the panels are showing.
        let mem_frac = snapshot.mem_used as f32 / snapshot.mem_total.max(1) as f32;
        let cpu_temperature = snapshot.temps.iter().find(|t| t.label == "CPU").or_else(|| snapshot.temps.first()).map(|t| t.degrees);
        let raised = self.alarms.evaluate(cpu_temperature.filter(|_| self.processor_temperature_warnings), mem_frac, &snapshot.filesystems, &snapshot.failed_units);
        self.alerts.extend(raised);
        while self.alerts.len() > PENDING_ALERTS {
            self.alerts.remove(0);
        }
        // What only this machine has always comes from it: power, the screen, sound.
        self.power = snapshot.power;
        self.battery = snapshot.battery;
        self.brightness = snapshot.brightness;
        self.wifi = snapshot.wifi;
        self.battery_health = snapshot.battery_health;
        if self.remote.as_ref().is_some_and(|remote| remote.live) {
            self.local = Some(snapshot);
            return;
        }
        self.local = Some(snapshot.clone());
        self.show_local(snapshot);
    }

    /// Puts this machine's readings in view.
    fn show_local(&mut self, snapshot: Snapshot) {
        (self.host, self.os, self.kernel) = self.local_names.clone();
        self.cpu_hist = snapshot.cpu_hist;
        self.cores = snapshot.cores;
        self.core_hist = snapshot.core_hist;
        self.mem_hist = snapshot.mem_hist;
        self.load = snapshot.load;
        self.temps = snapshot.temps;
        self.fans = snapshot.fans;
        self.gpu = snapshot.gpu;
        self.gpu_hist = snapshot.gpu_hist;
        self.disk_read_hist = snapshot.disk_read_hist;
        self.disk_write_hist = snapshot.disk_write_hist;
        self.disk_read_total = snapshot.disk_read_total;
        self.disk_write_total = snapshot.disk_write_total;
        self.filesystems = snapshot.filesystems;
        self.battery_health = snapshot.battery_health;
        self.failed_units = snapshot.failed_units;
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
    }

    /// Shows another machine's readings in place of this one's, or this one's again
    /// with `None`. Local sampling and its alarms carry on underneath either way, and
    /// what only this machine has (battery, screen, sound) stays this machine's.
    /// A host that is followed but not answering keeps this machine's readings in view.
    pub fn set_remote(&mut self, following: Option<Following>, view: Option<&crate::remote::View>) {
        let was_live = self.remote.as_ref().is_some_and(|remote| remote.live);
        let view = view.filter(|_| following.as_ref().is_some_and(|f| f.live));
        self.remote = following;
        match view {
            Some(view) => {
                self.host = view.host.clone();
                self.os = view.os.clone();
                self.kernel = view.kernel.clone();
                self.remote_uptime = Some(view.uptime);
                self.cpu_hist = view.cpu_hist.clone();
                self.cores = view.cores.clone();
                self.core_hist = view.core_hist.clone();
                self.mem_hist = view.mem_hist.clone();
                self.load = view.load;
                self.mem_used = view.mem_used;
                self.mem_total = view.mem_total;
                self.swap_used = view.swap_used;
                self.swap_total = view.swap_total;
                self.top = view.top.iter().map(|p| ProcInfo { name: p.name.clone(), cpu: p.cpu, mem: p.mem }).collect();
                self.iface = view.iface.clone();
                self.ip = view.ip.clone();
                self.rx_hist = view.rx_hist.clone();
                self.tx_hist = view.tx_hist.clone();
                self.rx_total = view.rx_total;
                self.tx_total = view.tx_total;
                self.filesystems = view.filesystems.clone();
                // Not read over ssh: these sections fold away rather than show this machine's.
                self.temps.clear();
                self.fans.clear();
                self.gpu = None;
                self.failed_units.clear();
                self.disk_read_hist = VecDeque::from(vec![0.0; HISTORY]);
                self.disk_write_hist = VecDeque::from(vec![0.0; HISTORY]);
                self.disk_read_total = 0;
                self.disk_write_total = 0;
            }
            None => {
                self.remote_uptime = None;
                if was_live {
                    if let Some(local) = self.local.clone() {
                        self.show_local(local);
                    } else {
                        (self.host, self.os, self.kernel) = self.local_names.clone();
                    }
                }
            }
        }
    }

    /// Time since boot of the machine in view.
    pub fn uptime_shown(&self) -> u64 {
        self.remote_uptime.unwrap_or_else(SysMon::uptime)
    }

    /// The processor's temperature, or failing a sensor of its own the hottest there is.
    #[cfg(test)]
    pub fn cpu_temperature(&self) -> Option<f32> {
        self.temps
            .iter()
            .find(|t| t.label == "CPU")
            .or_else(|| self.temps.first())
            .map(|t| t.degrees)
    }

    /// Alerts raised since the last call, oldest first; each is raised once until its
    /// reading has come back down.
    pub fn take_alerts(&mut self) -> Vec<Alert> {
        std::mem::take(&mut self.alerts)
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

/// Something the user should hear about, as the notification centre shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertKind { ProcessorTemperature, Memory, Disk, Service }

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub kind: AlertKind,
    pub title: String,
    pub body: String,
    pub urgent: bool,
}

/// Raised once when a reading passes `raise_at`, and not again until it has fallen
/// below `clear_below`, so a reading hovering around the line does not nag.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Latch(bool);

impl Latch {
    fn update(&mut self, value: f32, raise_at: f32, clear_below: f32) -> bool {
        if self.0 {
            if value < clear_below {
                self.0 = false;
            }
            false
        } else if value >= raise_at {
            self.0 = true;
            true
        } else {
            false
        }
    }
}

/// The lines readings must not cross, and which of them have.
#[derive(Debug, Default)]
pub struct Alarms {
    temperature: Latch,
    memory: Latch,
    disks: HashMap<String, Latch>,
    failed: HashSet<String>,
}

impl Alarms {
    pub const HOT: f32 = 90.0;
    pub const COOLED: f32 = 80.0;
    pub const DISK_FULL: f32 = 0.95;
    pub const DISK_CLEARED: f32 = 0.90;
    pub const MEMORY_FULL: f32 = 0.95;
    pub const MEMORY_CLEARED: f32 = 0.85;

    pub fn evaluate(&mut self, cpu_temperature: Option<f32>, memory: f32, filesystems: &[Filesystem], failed: &[String]) -> Vec<Alert> {
        let mut raised = Vec::new();
        if let Some(degrees) = cpu_temperature {
            if self.temperature.update(degrees, Self::HOT, Self::COOLED) {
                raised.push(Alert {
                    kind: AlertKind::ProcessorTemperature,
                    title: "Processor running hot".into(),
                    body: format!("{degrees:.0} °C. Check what is working it, and that the air can get through."),
                    urgent: true,
                });
            }
        }
        if self.memory.update(memory, Self::MEMORY_FULL, Self::MEMORY_CLEARED) {
            raised.push(Alert {
                kind: AlertKind::Memory,
                    title: "Memory nearly full".into(),
                body: format!("{:.0}% in use. The machine may start to swap and slow down.", memory * 100.0),
                urgent: true,
            });
        }
        for fs in filesystems {
            let latch = self.disks.entry(fs.mount.clone()).or_default();
            if latch.update(fs.used, Self::DISK_FULL, Self::DISK_CLEARED) {
                raised.push(Alert {
                    kind: AlertKind::Disk,
                    title: "Disk nearly full".into(),
                    body: format!("{} is {:.0}% full.", fs.mount, fs.used * 100.0),
                    urgent: false,
                });
            }
        }
        self.disks.retain(|mount, _| filesystems.iter().any(|fs| &fs.mount == mount));
        for unit in failed {
            if self.failed.insert(unit.clone()) {
                raised.push(Alert {
                    kind: AlertKind::Service,
                    title: "A service failed".into(),
                    body: format!("{unit} has failed. `systemctl status {unit}` says why."),
                    urgent: false,
                });
            }
        }
        self.failed.retain(|unit| failed.contains(unit));
        raised
    }
}

#[derive(Clone)]
struct Snapshot {
    cpu_hist: VecDeque<f32>,
    cores: Vec<f32>,
    core_hist: Vec<VecDeque<f32>>,
    mem_hist: VecDeque<f32>,
    load: Option<[f32; 3]>,
    temps: Vec<Temperature>,
    fans: Vec<Fan>,
    gpu: Option<Gpu>,
    gpu_hist: VecDeque<f32>,
    disk_read_hist: VecDeque<f32>,
    disk_write_hist: VecDeque<f32>,
    disk_read_total: u64,
    disk_write_total: u64,
    filesystems: Vec<Filesystem>,
    battery_health: Option<u8>,
    failed_units: Vec<String>,
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
            core_hist: Vec::new(),
            mem_hist: VecDeque::from(vec![0.0; HISTORY]),
            load: None,
            temps: Vec::new(),
            fans: Vec::new(),
            gpu: None,
            gpu_hist: VecDeque::from(vec![0.0; HISTORY]),
            disk_read_hist: VecDeque::from(vec![0.0; HISTORY]),
            disk_write_hist: VecDeque::from(vec![0.0; HISTORY]),
            disk_read_total: 0,
            disk_write_total: 0,
            filesystems: Vec::new(),
            battery_health: None,
            failed_units: Vec::new(),
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
    disks: Disks,
    last: Option<Instant>,
    diskstats: Vec<(String, u64, u64)>,
    /// Programs asked now and then, found once; absent programs are not looked for again.
    systemctl: Option<PathBuf>,
    nvidia_smi: Option<PathBuf>,
    units_at: Option<Instant>,
    nvidia_at: Option<Instant>,
    snapshot: Snapshot,
}

impl Sampler {
    fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        let nets = Networks::new_with_refreshed_list();
        Self {
            sys,
            nets,
            disks: Disks::new_with_refreshed_list(),
            last: None,
            diskstats: sensors::read_diskstats(),
            systemctl: sensors::find_program("systemctl"),
            nvidia_smi: sensors::find_program("nvidia-smi"),
            units_at: None,
            nvidia_at: None,
            snapshot: Snapshot::default(),
        }
    }

    /// Everything beyond sysinfo: sensors, disks, the load and the units.
    fn sample_sensors(&mut self, now: Instant, elapsed: Option<Duration>) {
        let devices = sensors::read_hwmon();
        self.snapshot.temps = sensors::temperatures(&devices);
        self.snapshot.fans = sensors::fans(&devices);
        self.snapshot.load = sensors::read_loadavg();
        self.snapshot.battery_health = sensors::read_battery_health();

        let stats = sensors::read_diskstats();
        let (read, written) = match elapsed {
            Some(elapsed) => sensors::disk_rates(&self.diskstats, &stats, elapsed),
            None => (0.0, 0.0),
        };
        push(&mut self.snapshot.disk_read_hist, read);
        push(&mut self.snapshot.disk_write_hist, written);
        self.snapshot.disk_read_total = stats.iter().map(|(_, r, _)| r).sum();
        self.snapshot.disk_write_total = stats.iter().map(|(_, _, w)| w).sum();
        self.diskstats = stats;

        self.disks.refresh(true);
        let mut filesystems: Vec<Filesystem> = self
            .disks
            .list()
            .iter()
            .filter(|disk| disk.total_space() > 0)
            .map(|disk| Filesystem {
                mount: disk.mount_point().to_string_lossy().into_owned(),
                used: 1.0 - disk.available_space() as f32 / disk.total_space() as f32,
            })
            .collect();
        filesystems.sort_by(|a, b| b.used.total_cmp(&a.used));
        filesystems.dedup_by(|a, b| a.mount == b.mount);
        filesystems.truncate(FILESYSTEMS);
        self.snapshot.filesystems = filesystems;

        // A GPU that tells the kernel is read every time; nvidia-smi is a program.
        let drm = sensors::read_drm_gpu_at(std::path::Path::new("/sys/class/drm"));
        let due = |at: Option<Instant>, every: Duration| at.is_none_or(|at| now.duration_since(at) >= every);
        let gpu = match (drm, &self.nvidia_smi) {
            (Some(gpu), _) => Some(gpu),
            (None, Some(program)) if due(self.nvidia_at, NVIDIA_EVERY) => {
                self.nvidia_at = Some(now);
                sensors::run_bounded(program, &["--query-gpu=name,utilization.gpu", "--format=csv,noheader,nounits"], Duration::from_secs(1))
                    .and_then(|text| sensors::parse_nvidia_smi(&text))
            }
            (None, Some(_)) => self.snapshot.gpu.clone(),
            (None, None) => None,
        };
        push(&mut self.snapshot.gpu_hist, gpu.as_ref().map_or(0.0, |g| g.busy as f32));
        self.snapshot.gpu = gpu;

        if let Some(systemctl) = &self.systemctl {
            if due(self.units_at, UNITS_EVERY) {
                self.units_at = Some(now);
                if let Some(units) = sensors::read_failed_units(systemctl) {
                    self.snapshot.failed_units = units;
                }
            }
        }
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
        let kept = self.snapshot.cores.len().min(CORE_HISTORIES);
        self.snapshot.core_hist.resize_with(kept, || VecDeque::from(vec![0.0; HISTORY]));
        for (hist, usage) in self.snapshot.core_hist.iter_mut().zip(&self.snapshot.cores) {
            push(hist, *usage);
        }
        self.snapshot.mem_used = self.sys.used_memory();
        self.snapshot.mem_total = self.sys.total_memory();
        push(&mut self.snapshot.mem_hist, self.snapshot.mem_used as f32 / self.snapshot.mem_total.max(1) as f32 * 100.0);
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
        self.sample_sensors(now, elapsed);
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
    fn processor_warning_opt_out_keeps_other_alarms_and_readings() {
        let mut mon = SysMon::new();
        mon.set_processor_temperature_warnings(false);
        let mut snapshot = Snapshot::default();
        snapshot.temps = vec![Temperature { label: "CPU".into(), degrees: 95.0 }];
        snapshot.mem_total = 100;
        snapshot.mem_used = 96;
        snapshot.filesystems = vec![Filesystem { mount: "/".into(), used: 0.96 }];
        snapshot.failed_units = vec!["example.service".into()];
        *mon.latest.lock().unwrap() = Some(snapshot.clone());
        mon.tick();
        let alerts = mon.take_alerts();
        assert_eq!(alerts.len(), 3);
        assert!(alerts.iter().all(|alert| alert.kind != AlertKind::ProcessorTemperature));
        assert_eq!(mon.cpu_temperature(), Some(95.0));
        mon.set_processor_temperature_warnings(true);
        *mon.latest.lock().unwrap() = Some(snapshot.clone());
        mon.tick();
        assert_eq!(mon.take_alerts().iter().filter(|alert| alert.kind == AlertKind::ProcessorTemperature).count(), 1);
        mon.set_processor_temperature_warnings(false);
        *mon.latest.lock().unwrap() = Some(snapshot);
        mon.tick();
        assert!(mon.take_alerts().is_empty());
    }

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
    fn a_followed_host_takes_the_panels_and_hands_them_back_with_local_history() {
        // A monitor fed by hand: no worker, one slot it reads from on each tick.
        let (stop, _) = mpsc::channel();
        let mut mon = SysMon::new();
        mon.latest = Arc::new(Mutex::new(None));
        mon._stop = stop;
        let local = |used: u64| {
            let mut snapshot = Snapshot::default();
            snapshot.mem_total = 100;
            snapshot.mem_used = used;
            push(&mut snapshot.cpu_hist, used as f32);
            snapshot.battery = Some(80);
            snapshot.iface = "wlan0".into();
            snapshot
        };
        let feed = |mon: &mut SysMon, snapshot: Snapshot| {
            *mon.latest.lock().unwrap() = Some(snapshot);
            mon.tick();
        };
        feed(&mut mon, local(10));
        let host = mon.host.clone();
        assert_eq!(mon.mem_used, 10);

        let mut view = crate::remote::View::default();
        view.host = "build".into();
        view.mem_total = 1000;
        view.mem_used = 900;
        view.iface = "eth0".into();
        view.uptime = 42;
        let live = || Some(Following { name: "me@build".into(), live: true, word: "" });

        // Connecting: still this machine in view.
        mon.set_remote(Some(Following { name: "me@build".into(), live: false, word: "CONNECTING" }), None);
        assert_eq!(mon.mem_used, 10);
        assert_eq!(mon.host, host);

        mon.set_remote(live(), Some(&view));
        assert_eq!((mon.host.as_str(), mon.mem_used, mon.iface.as_str()), ("build", 900, "eth0"));
        assert_eq!(mon.uptime_shown(), 42);
        // This machine keeps sampling underneath; its battery stays in view, its memory does not.
        feed(&mut mon, local(20));
        assert_eq!(mon.mem_used, 900, "a local sample does not push the host aside");
        assert_eq!(mon.battery, Some(80));

        mon.set_remote(None, None);
        assert_eq!(mon.host, host);
        assert_eq!(mon.mem_used, 20, "the latest local sample comes straight back");
        assert_eq!(mon.iface, "wlan0");
        let recent: Vec<f32> = mon.cpu_hist.iter().rev().take(1).copied().collect();
        assert_eq!(recent, vec![20.0], "local history kept while the host was shown");
        assert_ne!(mon.uptime_shown(), 42);
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
    fn alarms_raise_once_and_need_the_reading_to_come_down() {
        let mut alarms = Alarms::default();
        let fs = |used: f32| vec![Filesystem { mount: "/".into(), used }];
        assert!(alarms.evaluate(Some(70.0), 0.5, &fs(0.5), &[]).is_empty());
        let hot = alarms.evaluate(Some(91.0), 0.5, &fs(0.5), &[]);
        assert_eq!(hot.len(), 1);
        assert!(hot[0].urgent && hot[0].title.contains("hot"));
        assert!(alarms.evaluate(Some(95.0), 0.5, &fs(0.5), &[]).is_empty(), "still hot: no repeat");
        assert!(alarms.evaluate(Some(85.0), 0.5, &fs(0.5), &[]).is_empty(), "cooling but not cleared");
        assert!(alarms.evaluate(Some(79.0), 0.5, &fs(0.5), &[]).is_empty(), "clearing raises nothing");
        assert_eq!(alarms.evaluate(Some(90.0), 0.5, &fs(0.5), &[]).len(), 1, "hot again: raised again");

        let full = alarms.evaluate(None, 0.96, &fs(0.96), &[]);
        assert_eq!(full.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(), ["Memory nearly full", "Disk nearly full"]);
        assert!(alarms.evaluate(None, 0.96, &fs(0.96), &[]).is_empty());
        assert!(alarms.evaluate(None, 0.80, &fs(0.80), &[]).is_empty());
        assert_eq!(alarms.evaluate(None, 0.95, &fs(0.95), &[]).len(), 2);
        assert!(alarms.evaluate(None, 0.0, &[], &[]).is_empty());
        assert!(alarms.disks.is_empty(), "an unmounted filesystem is forgotten");

        let failed = vec!["foo.service".to_string()];
        assert_eq!(alarms.evaluate(None, 0.0, &[], &failed).len(), 1);
        assert!(alarms.evaluate(None, 0.0, &[], &failed).is_empty(), "a unit that stays failed is told once");
        assert!(alarms.evaluate(None, 0.0, &[], &[]).is_empty());
        assert_eq!(alarms.evaluate(None, 0.0, &[], &failed).len(), 1, "failing again is news again");
    }

    #[test]
    fn tick_queues_alerts_until_taken_and_bounds_them() {
        let mut mon = SysMon::new();
        assert!(mon.take_alerts().is_empty());
        for i in 0..(PENDING_ALERTS + 4) {
            mon.alerts.push(Alert { kind: AlertKind::Service, title: format!("{i}"), body: String::new(), urgent: false });
        }
        let mut snapshot = Snapshot::default();
        snapshot.mem_total = 100;
        snapshot.mem_used = 96;
        // A slot of its own, so the real sampler cannot overwrite the planted sample.
        mon.latest = Arc::new(Mutex::new(Some(snapshot)));
        mon.tick();
        let alerts = mon.take_alerts();
        assert_eq!(alerts.len(), PENDING_ALERTS);
        assert_eq!(alerts.last().unwrap().title, "Memory nearly full", "the newest alert is kept");
        assert_eq!(alerts[0].title, "5", "the oldest are dropped");
        assert!(mon.take_alerts().is_empty());
        assert_eq!(mon.cpu_temperature(), None);
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
        assert_eq!(mon.core_hist.len(), mon.cores.len().min(CORE_HISTORIES));
        assert!(mon.core_hist.iter().all(|h| h.len() == HISTORY));
        assert_eq!(mon.mem_hist.len(), HISTORY);
        assert!(mon.mem_hist.back().is_some_and(|m| (0.0..=100.0).contains(m)));
        assert!(mon.filesystems.iter().all(|fs| (0.0..=1.0).contains(&fs.used)));
        assert!(mon.temps.windows(2).all(|w| w[0].degrees >= w[1].degrees), "hottest first");
    }
}
