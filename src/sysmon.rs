//! What the system monitor shows: the processor, memory and network figures and the process list, read from `/proc`.
//! The parsers take text so they can be tried on samples; `Sampler` reads the files and keeps what it needs from the
//! last look to turn counters into rates (a process's share of the processor, bytes a second on the network).

use std::collections::HashMap;
use std::time::Instant;

/// One `cpu` line of `/proc/stat`: (all the time counted, the idle part of it).
pub fn parse_stat_cpus(text: &str) -> Vec<(u64, u64)> {
    text.lines()
        .filter(|l| l.starts_with("cpu"))
        .filter_map(|l| {
            let v: Vec<u64> = l.split_whitespace().skip(1).filter_map(|n| n.parse().ok()).collect();
            if v.len() < 5 {
                return None;
            }
            // user nice system idle iowait irq softirq steal ...: guest time is already inside user
            let total: u64 = v.iter().take(8).sum();
            Some((total, v[3] + v[4]))
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemInfo {
    pub total: u64,
    pub available: u64,
    pub cached: u64,
    pub buffers: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

impl MemInfo {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn swap_used(&self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }
}

/// `/proc/meminfo` (kB in the file, bytes here).
pub fn parse_meminfo(text: &str) -> MemInfo {
    let mut m = MemInfo::default();
    for l in text.lines() {
        let Some((k, v)) = l.split_once(':') else { continue };
        let n: u64 = v.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0) * 1024;
        match k {
            "MemTotal" => m.total = n,
            "MemAvailable" => m.available = n,
            "Cached" => m.cached = n,
            "Buffers" => m.buffers = n,
            "SwapTotal" => m.swap_total = n,
            "SwapFree" => m.swap_free = n,
            _ => {}
        }
    }
    m
}

/// The bytes received and sent over every interface but the loopback, from `/proc/net/dev`.
pub fn parse_net_dev(text: &str) -> (u64, u64) {
    let (mut rx, mut tx) = (0u64, 0u64);
    for l in text.lines().skip(2) {
        let Some((name, rest)) = l.split_once(':') else { continue };
        if name.trim() == "lo" {
            continue;
        }
        let v: Vec<u64> = rest.split_whitespace().filter_map(|n| n.parse().ok()).collect();
        if v.len() >= 9 {
            rx += v[0];
            tx += v[8];
        }
    }
    (rx, tx)
}

/// What `/proc/<pid>/stat` says about a process.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcRaw {
    pub pid: u32,
    pub name: String,
    pub state: char,
    pub ppid: u32,
    /// user + system time, in clock ticks
    pub ticks: u64,
    pub threads: u32,
    pub rss_pages: u64,
}

/// `pid (name) S ppid ...`. The name is in brackets and may hold spaces and brackets of its own, so the fields are counted
/// from the last closing bracket.
pub fn parse_proc_stat(text: &str) -> Option<ProcRaw> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let pid = text[..open].trim().parse().ok()?;
    let name = text[open + 1..close].to_string();
    let f: Vec<&str> = text[close + 1..].split_whitespace().collect();
    // f[0] state, f[1] ppid, f[11] utime, f[12] stime, f[17] num_threads, f[21] rss
    if f.len() < 22 {
        return None;
    }
    Some(ProcRaw { pid, name, state: f[0].chars().next()?, ppid: f[1].parse().ok()?, ticks: f[11].parse::<u64>().ok()? + f[12].parse::<u64>().ok()?, threads: f[17].parse().ok()?, rss_pages: f[21].parse().ok()? })
}

/// A name the kernel cut at 15 characters, made whole from the first word of the command line when that begins the same
/// way (at least four letters alike: a wrapper may start the program under a shorter name);
/// the wrapper a Nix program is started through (`.name-wrapped`) is not shown.
pub fn full_name(comm: &str, cmdline: &str) -> String {
    let mut name = comm.to_string();
    if comm.chars().count() >= 15 {
        let arg0 = cmdline.split('\0').next().unwrap_or("");
        let base = arg0.rsplit('/').next().unwrap_or("");
        let plain = |s: &str| s.trim_start_matches('.').to_lowercase();
        let alike = plain(base).chars().zip(plain(comm).chars()).take_while(|(a, b)| a == b).count();
        if !base.is_empty() && alike >= 4 {
            name = base.to_string();
        }
    }
    let n = name.strip_prefix('.').unwrap_or(&name);
    let n = n.strip_suffix("-wrapped").unwrap_or(n);
    if n.is_empty() { name } else { n.to_string() }
}

/// The real user id from `/proc/<pid>/status`.
pub fn parse_status_uid(text: &str) -> Option<u32> {
    text.lines().find_map(|l| l.strip_prefix("Uid:")).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse().ok())
}

/// uid -> name from the text of `/etc/passwd`.
pub fn parse_passwd(text: &str) -> HashMap<u32, String> {
    text.lines().filter_map(|l| {
        let mut f = l.split(':');
        let name = f.next()?;
        let uid = f.nth(1)?.parse().ok()?;
        Some((uid, name.to_string()))
    }).collect()
}

/// Context switches (voluntary + not) from `/proc/<pid>/status`. Each time a sleeping process is woken it counts one, so the
/// difference over a second is how often the process wakes the processor.
pub fn parse_ctxt_switches(text: &str) -> u64 {
    text.lines()
        .filter(|l| l.starts_with("voluntary_ctxt_switches:") || l.starts_with("nonvoluntary_ctxt_switches:"))
        .filter_map(|l| l.split(':').nth(1)?.trim().parse::<u64>().ok())
        .sum()
}

/// What a DRM file descriptor's `/proc/<pid>/fdinfo/<fd>` says: the client id (one process may open the card many times),
/// the PCI address of the card and the time its engines have worked for it, in nanoseconds (drivers that report it: i915, xe, amdgpu).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrmFd {
    pub client: Option<u64>,
    pub pdev: String,
    pub engine_ns: u64,
}

pub fn parse_drm_fdinfo(text: &str) -> Option<DrmFd> {
    let mut d = DrmFd::default();
    let mut drm = false;
    for l in text.lines() {
        let Some((k, v)) = l.split_once(':') else { continue };
        let v = v.trim();
        if k == "drm-driver" {
            drm = true;
        } else if k == "drm-client-id" {
            d.client = v.parse().ok();
        } else if k == "drm-pdev" {
            d.pdev = v.to_string();
        } else if k.starts_with("drm-engine-") && !k.starts_with("drm-engine-capacity-") {
            d.engine_ns += v.split_whitespace().next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
        }
    }
    drm.then_some(d)
}

/// Does the card at this PCI address (`0000:00:02.0`) sit on the processor's own bus: the integrated one?
fn pdev_is_integrated(pdev: &str) -> bool {
    pdev.split(':').nth(1) == Some("00")
}

/// The graphics card a process has open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuUse {
    #[default]
    None,
    Integrated,
    /// holds the dedicated card open: while it does, the card cannot power down
    Dedicated,
}

/// Look at the file descriptors of a process: the dedicated card (`/dev/nvidia0`...) or a DRM node of either card.
/// Returns what it holds and the engine time (ns) its integrated-card clients have used. `None` when the process
/// cannot be looked into (another user's).
fn scan_gpu(pid: u32) -> Option<(GpuUse, u64)> {
    let rd = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let (mut using, mut engine) = (GpuUse::None, 0u64);
    let mut seen: Vec<u64> = Vec::new();
    for e in rd.flatten() {
        let Ok(target) = std::fs::read_link(e.path()) else { continue };
        let t = target.to_string_lossy();
        if let Some(rest) = t.strip_prefix("/dev/nvidia") {
            // nvidia0, nvidia1...; the control nodes (nvidiactl, nvidia-modeset, nvidia-uvm) say nothing about drawing
            if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                using = GpuUse::Dedicated;
            }
        } else if t.starts_with("/dev/dri/") {
            let info = std::fs::read_to_string(format!("/proc/{pid}/fdinfo/{}", e.file_name().to_string_lossy())).unwrap_or_default();
            let Some(d) = parse_drm_fdinfo(&info) else { continue };
            if let Some(c) = d.client {
                if seen.contains(&c) {
                    continue;
                }
                seen.push(c);
            }
            if d.pdev.is_empty() || pdev_is_integrated(&d.pdev) {
                if using == GpuUse::None {
                    using = GpuUse::Integrated;
                }
                engine += d.engine_ns;
            } else {
                using = GpuUse::Dedicated;
            }
        }
    }
    Some((using, engine))
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub user: String,
    pub state: char,
    /// share of the processor in percent; 100 is one whole core
    pub cpu: f32,
    pub mem: u64,
    pub threads: u32,
    /// how often a second the process was woken
    pub wakeups: f32,
    pub gpu: GpuUse,
    /// share of the integrated card's engines in percent, for programs that draw there
    pub gpu_busy: f32,
    /// a rough cost in energy: see `energy_impact`
    pub energy: f32,
}

/// The weight of a wake-up in the energy figure: a process woken 100 times a second costs as much as 5 % of a core.
const WAKEUP_COST: f32 = 0.05;
/// What holding the dedicated card open costs in the figure, in percent of a core: the card cannot sleep while anyone has it.
const DEDICATED_COST: f32 = 25.0;
/// Percent of the integrated card's engines, in percent of a core.
const IGPU_COST: f32 = 0.4;

/// A figure in the spirit of macOS's "Energy Impact": processor time, wake-ups, and the graphics card the process keeps busy
/// (or merely awake). It ranks processes against each other; it is not a number of watts.
pub fn energy_impact(cpu: f32, wakeups: f32, gpu: GpuUse, gpu_busy: f32) -> f32 {
    cpu + wakeups * WAKEUP_COST + gpu_busy * IGPU_COST + if gpu == GpuUse::Dedicated { DEDICATED_COST } else { 0.0 }
}

impl ProcInfo {
    /// The state as a word.
    pub fn state_name(&self) -> &'static str {
        match self.state {
            'R' => "Running",
            'S' => "Sleeping",
            'D' => "Waiting",
            'T' | 't' => "Stopped",
            'Z' => "Zombie",
            'I' => "Idle",
            _ => "Other",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// the whole processor, and each core, in percent (0 .. 100)
    pub cpu: f32,
    pub cores: Vec<f32>,
    pub mem: MemInfo,
    /// bytes a second
    pub rx: f64,
    pub tx: f64,
    pub procs: Vec<ProcInfo>,
    pub load: [f32; 3],
    pub uptime: u64,
    /// the temperature sensors of the machine, the processor's first
    pub temps: Vec<Sensor>,
    pub battery: Option<Battery>,
    /// the graphics cards and whether each one is powered down now
    pub gpus: Vec<crate::gpu::Gpu>,
}

/// The battery: what it gives (or takes) and its charge.
#[derive(Clone, Debug, PartialEq)]
pub struct Battery {
    /// watts flowing in or out; 0 when nothing flows
    pub watts: f32,
    pub percent: u8,
    /// `Discharging`, `Charging`, `Full`, `Not charging`...
    pub status: String,
}

impl Battery {
    pub fn discharging(&self) -> bool {
        self.status == "Discharging"
    }
}

/// The first battery under `root` (`/sys/class/power_supply`). The draw is `power_now` (µW), or current x voltage when the
/// driver gives those instead.
pub fn read_battery_from(root: &std::path::Path) -> Option<Battery> {
    let mut dirs: Vec<_> = std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for d in dirs {
        let f = |n: &str| read(&d.join(n).to_string_lossy()).trim().to_string();
        if f("type") != "Battery" {
            continue;
        }
        let num = |n: &str| f(n).parse::<f64>().ok();
        let micro_w = match num("power_now") {
            Some(p) if p > 0.0 => p,
            _ => num("current_now").zip(num("voltage_now")).map_or(0.0, |(i, v)| i * v / 1e6),
        };
        return Some(Battery { watts: (micro_w / 1e6) as f32, percent: num("capacity").unwrap_or(0.0).clamp(0.0, 100.0) as u8, status: f("status") });
    }
    None
}

/// One temperature sensor: a readable name and degrees Celsius.
#[derive(Clone, Debug, PartialEq)]
pub struct Sensor {
    pub name: String,
    pub celsius: f32,
}

/// A readable name for a `hwmon` sensor from its chip (`coretemp`, `nvme`...) and its label (`Package id 0`...).
pub fn sensor_name(chip: &str, label: &str) -> String {
    let label = label.trim();
    match chip {
        "coretemp" | "k10temp" | "zenpower" => {
            if let Some(n) = label.strip_prefix("Core ") {
                format!("CPU core {n}")
            } else if label.starts_with("Package") || label == "Tctl" || label == "Tdie" || label.is_empty() {
                "CPU package".to_string()
            } else {
                format!("CPU {label}")
            }
        }
        "nvme" => if label == "Composite" || label.is_empty() { "NVMe".to_string() } else { format!("NVMe {}", label.to_lowercase()) },
        "acpitz" => "ACPI zone".to_string(),
        "spd5118" | "jc42" => "Memory module".to_string(),
        "amdgpu" | "nouveau" | "radeon" => if label.is_empty() || label == "edge" { "GPU".to_string() } else { format!("GPU {label}") },
        "iwlwifi_1" | "iwlwifi" => "Wi-Fi card".to_string(),
        _ if label.is_empty() => chip.to_string(),
        _ => format!("{chip} {label}"),
    }
}

/// Order for showing: the processor package and the graphics chip, the other sensors, the processor cores last.
fn sensor_rank(name: &str) -> u8 {
    if name == "CPU package" {
        0
    } else if name.starts_with("GPU") {
        1
    } else if name.starts_with("CPU core") {
        3
    } else {
        2
    }
}

/// Read every `temp*_input` under `root` (`/sys/class/hwmon`). Chips that share a name are told apart by a number.
pub fn read_temps_from(root: &std::path::Path) -> Vec<Sensor> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut chips: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    chips.sort_by_key(|p| {
        let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        n.trim_start_matches("hwmon").parse::<u32>().unwrap_or(u32::MAX)
    });
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut out: Vec<Sensor> = Vec::new();
    for dir in chips {
        let chip = read(&dir.join("name").to_string_lossy()).trim().to_string();
        let mut inputs: Vec<(u32, f32)> = Vec::new();
        if let Ok(files) = std::fs::read_dir(&dir) {
            for f in files.flatten() {
                let fname = f.file_name();
                let Some(n) = fname.to_str().and_then(|n| n.strip_prefix("temp")).and_then(|n| n.strip_suffix("_input")).and_then(|n| n.parse::<u32>().ok()) else { continue };
                let Some(milli) = std::fs::read_to_string(f.path()).ok().and_then(|t| t.trim().parse::<i64>().ok()) else { continue };
                let c = milli as f32 / 1000.0;
                // a sensor that reads nothing sensible is left out
                if c > 0.0 && c < 150.0 {
                    inputs.push((n, c));
                }
            }
        }
        if inputs.is_empty() {
            continue;
        }
        inputs.sort_by_key(|i| i.0);
        let nth = seen.entry(chip.clone()).and_modify(|n| *n += 1).or_insert(1);
        let nth = *nth;
        for (n, c) in inputs {
            let label = read(&dir.join(format!("temp{n}_label")).to_string_lossy());
            let mut name = sensor_name(&chip, &label);
            if nth > 1 {
                name = format!("{name} {nth}");
            }
            out.push(Sensor { name, celsius: c });
        }
    }
    out.sort_by_key(|s| sensor_rank(&s.name));
    out
}

pub struct Sampler {
    prev_cpu: Vec<(u64, u64)>,
    prev_ticks: HashMap<u32, u64>,
    prev_net: Option<(u64, u64, Instant)>,
    users: HashMap<u32, String>,
    page: u64,
    prev_ctxt: HashMap<u32, u64>,
    prev_look: Option<Instant>,
    /// what each process held open at the last graphics look, and its engine time then
    gpu_seen: HashMap<u32, (GpuUse, u64)>,
    gpu_busy: HashMap<u32, f32>,
    gpu_look: Option<Instant>,
    looks: u32,
    gpus: Vec<crate::gpu::Gpu>,
}

/// The file descriptors of every process are read every this many looks, not every one.
const GPU_EVERY: u32 = 2;

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

impl Sampler {
    pub fn new() -> Sampler {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64;
        Sampler { prev_cpu: Vec::new(), prev_ticks: HashMap::new(), prev_net: None, users: parse_passwd(&read("/etc/passwd")), page, prev_ctxt: HashMap::new(), prev_look: None, gpu_seen: HashMap::new(), gpu_busy: HashMap::new(), gpu_look: None, looks: 0, gpus: Vec::new() }
    }

    /// Look at the system now. The first look has no rates yet (they are zero).
    pub fn sample(&mut self) -> Snapshot {
        let cpus = parse_stat_cpus(&read("/proc/stat"));
        let pct = |now: (u64, u64), before: Option<&(u64, u64)>| -> f32 {
            let Some(b) = before else { return 0.0 };
            let (dt, di) = (now.0.saturating_sub(b.0), now.1.saturating_sub(b.1));
            if dt == 0 { 0.0 } else { (100.0 * (dt - di.min(dt)) as f32 / dt as f32).clamp(0.0, 100.0) }
        };
        let cpu = cpus.first().map_or(0.0, |c| pct(*c, self.prev_cpu.first()));
        let cores: Vec<f32> = cpus.iter().enumerate().skip(1).map(|(i, c)| pct(*c, self.prev_cpu.get(i))).collect();
        // ticks that went by on the whole machine, to turn a process's ticks into a share
        let total_delta = cpus.first().zip(self.prev_cpu.first()).map_or(0, |(n, b)| n.0.saturating_sub(b.0)) as f32;
        let ncores = cores.len().max(1) as f32;

        let now = Instant::now();
        let dt = self.prev_look.map_or(0.0, |t| now.duration_since(t).as_secs_f32());
        // which process holds which card: the descriptors are read every second look (the dedicated card is never touched)
        let gpu_due = self.looks % GPU_EVERY == 0;
        let gpu_dt = self.gpu_look.map_or(0.0, |t| now.duration_since(t).as_secs_f32());
        if gpu_due {
            self.gpus = crate::gpu::detect();
            self.gpu_look = Some(now);
        }
        let mut gpu_now: HashMap<u32, (GpuUse, u64)> = HashMap::new();
        let mut ctxt_now = HashMap::new();
        let mut procs = Vec::new();
        let mut ticks_now = HashMap::new();
        if let Ok(rd) = std::fs::read_dir("/proc") {
            for e in rd.flatten() {
                let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
                let Some(raw) = parse_proc_stat(&read(&format!("/proc/{pid}/stat"))) else { continue };
                let status = read(&format!("/proc/{pid}/status"));
                let uid = parse_status_uid(&status).unwrap_or(u32::MAX);
                let ctxt = parse_ctxt_switches(&status);
                let wakeups = match self.prev_ctxt.get(&pid) {
                    Some(&before) if dt > 0.0 => ctxt.saturating_sub(before) as f32 / dt,
                    _ => 0.0,
                };
                ctxt_now.insert(pid, ctxt);
                let seen = if gpu_due { scan_gpu(pid) } else { self.gpu_seen.get(&pid).copied() };
                if let Some(g) = seen {
                    gpu_now.insert(pid, g);
                }
                let (gpu, gpu_busy) = match (seen, self.gpu_seen.get(&pid)) {
                    // engine time is a counter: its growth since the last graphics look, over the time between them
                    (Some((u, ns)), Some(&(_, before))) if gpu_due && gpu_dt > 0.0 => {
                        let busy = 100.0 * ns.saturating_sub(before) as f32 / (gpu_dt * 1e9);
                        self.gpu_busy.insert(pid, busy);
                        (u, busy)
                    }
                    (Some((u, _)), _) => (u, self.gpu_busy.get(&pid).copied().unwrap_or(0.0)),
                    (None, _) => (GpuUse::None, 0.0),
                };
                let name = if raw.name.chars().count() >= 15 { full_name(&raw.name, &read(&format!("/proc/{pid}/cmdline"))) } else { full_name(&raw.name, "") };
                let share = match self.prev_ticks.get(&pid) {
                    Some(&before) if total_delta > 0.0 => 100.0 * ncores * raw.ticks.saturating_sub(before) as f32 / total_delta,
                    _ => 0.0,
                };
                ticks_now.insert(pid, raw.ticks);
                procs.push(ProcInfo {
                    pid,
                    ppid: raw.ppid,
                    name,
                    user: self.users.get(&uid).cloned().unwrap_or_else(|| if uid == u32::MAX { "?".into() } else { uid.to_string() }),
                    state: raw.state,
                    cpu: share.max(0.0),
                    mem: raw.rss_pages * self.page,
                    threads: raw.threads,
                    wakeups,
                    gpu,
                    gpu_busy,
                    energy: energy_impact(share.max(0.0), wakeups, gpu, gpu_busy),
                });
            }
        }
        self.prev_cpu = cpus;
        self.prev_ticks = ticks_now;
        self.prev_ctxt = ctxt_now;
        self.prev_look = Some(now);
        if gpu_due {
            self.gpu_seen = gpu_now;
            let live = &self.gpu_seen;
            self.gpu_busy.retain(|pid, _| live.contains_key(pid));
        }
        self.looks = self.looks.wrapping_add(1);

        let (rx, tx) = parse_net_dev(&read("/proc/net/dev"));
        let (rx_rate, tx_rate) = match self.prev_net {
            Some((prx, ptx, t)) => {
                let dt = now.duration_since(t).as_secs_f64().max(0.001);
                (rx.saturating_sub(prx) as f64 / dt, tx.saturating_sub(ptx) as f64 / dt)
            }
            None => (0.0, 0.0),
        };
        self.prev_net = Some((rx, tx, now));

        let load: Vec<f32> = read("/proc/loadavg").split_whitespace().take(3).filter_map(|n| n.parse().ok()).collect();
        let uptime = read("/proc/uptime").split_whitespace().next().and_then(|n| n.parse::<f64>().ok()).unwrap_or(0.0) as u64;
        Snapshot { cpu, cores, mem: parse_meminfo(&read("/proc/meminfo")), rx: rx_rate, tx: tx_rate, procs, load: [*load.first().unwrap_or(&0.0), *load.get(1).unwrap_or(&0.0), *load.get(2).unwrap_or(&0.0)], uptime, temps: read_temps_from(std::path::Path::new("/sys/class/hwmon")), battery: read_battery_from(std::path::Path::new("/sys/class/power_supply")), gpus: self.gpus.clone() }
    }
}

/// Ask a process to end (`force`: make it) with a signal.
pub fn signal(pid: u32, force: bool) -> Result<(), String> {
    let r = unsafe { libc::kill(pid as i32, if force { libc::SIGKILL } else { libc::SIGTERM }) };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

/// "3 d 4 h", "2 h 05 min", "7 min".
pub fn format_uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    if d > 0 {
        format!("{d} d {h} h")
    } else if h > 0 {
        format!("{h} h {m:02} min")
    } else {
        format!("{m} min")
    }
}

/// "1.5 MB/s".
pub fn format_rate(bytes_per_s: f64) -> String {
    format!("{}/s", crate::fsops::human_size(bytes_per_s.max(0.0) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processor_lines_give_total_and_idle() {
        let t = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 60 0 20 400 20 0 0 0 0 0\ncpu1 40 0 30 400 30 0 0 0 0 0\nintr 1 2 3\n";
        let v = parse_stat_cpus(t);
        assert_eq!(v, vec![(1000, 850), (500, 420), (500, 430)]);
    }

    #[test]
    fn memory_and_swap_are_in_bytes() {
        let m = parse_meminfo("MemTotal:  16000 kB\nMemFree: 1000 kB\nMemAvailable:  9000 kB\nBuffers: 100 kB\nCached: 2000 kB\nSwapTotal: 4000 kB\nSwapFree: 3000 kB\n");
        assert_eq!((m.total, m.used(), m.cached, m.swap_used()), (16000 * 1024, 7000 * 1024, 2000 * 1024, 1000 * 1024));
    }

    #[test]
    fn the_network_skips_the_loopback() {
        let t = "Inter-|   Receive\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n    lo: 500 5 0 0 0 0 0 0 500 5 0 0 0 0 0 0\n  eth0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0\n wlan0: 10 1 0 0 0 0 0 0 20 2 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(t), (1010, 2020));
    }

    #[test]
    fn a_process_name_may_hold_brackets_and_spaces() {
        let s = "4242 (Web Content (x)) S 1 4242 4242 0 -1 4194560 1000 0 0 0 150 50 0 0 20 0 7 0 12345 1000000 321 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0";
        let p = parse_proc_stat(s).unwrap();
        assert_eq!((p.pid, p.name.as_str(), p.state, p.ppid, p.ticks, p.threads, p.rss_pages), (4242, "Web Content (x)", 'S', 1, 200, 7, 321));
        assert!(parse_proc_stat("garbage").is_none());
    }

    #[test]
    fn users_come_from_the_status_and_the_passwd_file() {
        assert_eq!(parse_status_uid("Name:\tx\nUid:\t1000\t1000\t1000\t1000\nGid:\t1\n"), Some(1000));
        let users = parse_passwd("root:x:0:0:root:/root:/bin/sh\nmykhailo:x:1000:100::/home/m:/bin/fish\n");
        assert_eq!((users[&0].as_str(), users[&1000].as_str()), ("root", "mykhailo"));
    }

    #[test]
    fn the_sampler_finds_this_process_and_a_share_of_the_processor() {
        let mut s = Sampler::new();
        let first = s.sample();
        assert!(first.mem.total > 0 && !first.cores.is_empty());
        // burn a little time so the second look has something to see
        let t = Instant::now();
        while t.elapsed().as_millis() < 150 {
            std::hint::black_box(1u64.wrapping_mul(3));
        }
        let second = s.sample();
        let me = second.procs.iter().find(|p| p.pid == std::process::id()).expect("this process is listed");
        assert!(me.mem > 0 && me.threads >= 1);
        assert!(me.cpu > 0.0, "a busy test process shows some processor use");
    }

    #[test]
    fn long_names_are_made_whole_and_wrappers_are_dropped() {
        assert_eq!(full_name(".plasmashell-wr", "/nix/store/x/bin/.plasmashell-wrapped\0--flag\0"), "plasmashell");
        assert_eq!(full_name(".plasmashell-wr", "/nix/store/y/bin/plasmashell\0"), "plasmashell");
        assert_eq!(full_name("ThreadPoolForeg", "/usr/bin/brave\0"), "ThreadPoolForeg");
        assert_eq!(full_name("bash", "/bin/bash\0-l\0"), "bash");
        assert_eq!(full_name("kworker/3:0-i915", ""), "kworker/3:0-i915");
        assert_eq!(full_name("abcdefghijklmno", "/usr/bin/abcdefghijklmnopq\0"), "abcdefghijklmnopq");
    }

    #[test]
    fn a_process_can_be_asked_to_end() {
        let mut child = std::process::Command::new("sleep").arg("30").spawn().expect("sleep is there");
        signal(child.id(), false).unwrap();
        let status = child.wait().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGTERM));
        // a process that is gone cannot be signalled
        assert!(signal(child.id(), true).is_err());
    }

    #[test]
    fn sensors_get_readable_names_and_the_processor_comes_first() {
        assert_eq!(sensor_name("coretemp", "Package id 0"), "CPU package");
        assert_eq!(sensor_name("coretemp", "Core 3"), "CPU core 3");
        assert_eq!(sensor_name("nvme", "Composite"), "NVMe");
        assert_eq!(sensor_name("nvme", "Sensor 2"), "NVMe sensor 2");
        assert_eq!(sensor_name("acpitz", ""), "ACPI zone");
        assert_eq!(sensor_name("weird", ""), "weird");

        let root = std::env::temp_dir().join(format!("memrium-hwmon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (dir, chip, files) in [("hwmon0", "nvme", vec![("1", Some("Composite"), "41850")]), ("hwmon1", "nvme", vec![("1", Some("Composite"), "39000")]), ("hwmon5", "coretemp", vec![("1", Some("Package id 0"), "55000"), ("2", Some("Core 0"), "52000"), ("3", None, "-1")])] {
            let d = root.join(dir);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("name"), format!("{chip}\n")).unwrap();
            for (n, label, v) in files {
                std::fs::write(d.join(format!("temp{n}_input")), format!("{v}\n")).unwrap();
                if let Some(l) = label {
                    std::fs::write(d.join(format!("temp{n}_label")), format!("{l}\n")).unwrap();
                }
            }
        }
        let t = read_temps_from(&root);
        let _ = std::fs::remove_dir_all(&root);
        let got: Vec<(&str, f32)> = t.iter().map(|s| (s.name.as_str(), s.celsius)).collect();
        assert_eq!(got, vec![("CPU package", 55.0), ("NVMe", 41.85), ("NVMe 2", 39.0), ("CPU core 0", 52.0)]);
    }

    #[test]
    fn wakeups_and_graphics_come_from_status_and_fdinfo() {
        assert_eq!(parse_ctxt_switches("Name:\tx\nvoluntary_ctxt_switches:\t120\nnonvoluntary_ctxt_switches:\t30\n"), 150);
        let d = parse_drm_fdinfo("pos:\t0\ndrm-driver:\ti915\ndrm-client-id:\t538\ndrm-pdev:\t0000:00:02.0\ndrm-engine-render:\t2000 ns\ndrm-engine-copy:\t500 ns\ndrm-engine-capacity-render:\t1\n").unwrap();
        assert_eq!((d.client, d.pdev.as_str(), d.engine_ns), (Some(538), "0000:00:02.0", 2500));
        assert!(pdev_is_integrated(&d.pdev) && !pdev_is_integrated("0000:01:00.0"));
        // a plain file descriptor is not a card
        assert!(parse_drm_fdinfo("pos:\t0\nflags:\t02\n").is_none());
    }

    #[test]
    fn the_dedicated_card_costs_more_than_a_busy_integrated_one() {
        let idle = energy_impact(0.0, 0.0, GpuUse::None, 0.0);
        let igpu = energy_impact(2.0, 40.0, GpuUse::Integrated, 30.0);
        let dgpu = energy_impact(2.0, 40.0, GpuUse::Dedicated, 0.0);
        assert_eq!(idle, 0.0);
        assert!(dgpu > igpu && igpu > idle, "{idle} {igpu} {dgpu}");
    }

    #[test]
    fn the_battery_draw_comes_from_power_or_current_and_voltage() {
        let root = std::env::temp_dir().join(format!("memrium-psu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (dir, files) in [("AC", vec![("type", "Mains")]), ("BAT0", vec![("type", "Battery"), ("status", "Discharging"), ("capacity", "83"), ("current_now", "1000000"), ("voltage_now", "12000000")]), ("BAT1", vec![("type", "Battery"), ("power_now", "9500000")])] {
            let d = root.join(dir);
            std::fs::create_dir_all(&d).unwrap();
            for (n, v) in files {
                std::fs::write(d.join(n), format!("{v}\n")).unwrap();
            }
        }
        let b = read_battery_from(&root).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!((b.percent, b.discharging()), (83, true));
        assert!((b.watts - 12.0).abs() < 0.01, "{}", b.watts);
    }

    #[test]
    fn times_and_rates_read_well() {
        assert_eq!(format_uptime(90061), "1 d 1 h");
        assert_eq!(format_uptime(7500), "2 h 05 min");
        assert_eq!(format_uptime(420), "7 min");
    }
}
