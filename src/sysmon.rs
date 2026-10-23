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
}

pub struct Sampler {
    prev_cpu: Vec<(u64, u64)>,
    prev_ticks: HashMap<u32, u64>,
    prev_net: Option<(u64, u64, Instant)>,
    users: HashMap<u32, String>,
    page: u64,
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

impl Sampler {
    pub fn new() -> Sampler {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64;
        Sampler { prev_cpu: Vec::new(), prev_ticks: HashMap::new(), prev_net: None, users: parse_passwd(&read("/etc/passwd")), page }
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

        let mut procs = Vec::new();
        let mut ticks_now = HashMap::new();
        if let Ok(rd) = std::fs::read_dir("/proc") {
            for e in rd.flatten() {
                let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
                let Some(raw) = parse_proc_stat(&read(&format!("/proc/{pid}/stat"))) else { continue };
                let uid = parse_status_uid(&read(&format!("/proc/{pid}/status"))).unwrap_or(u32::MAX);
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
                });
            }
        }
        self.prev_cpu = cpus;
        self.prev_ticks = ticks_now;

        let (rx, tx) = parse_net_dev(&read("/proc/net/dev"));
        let now = Instant::now();
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
        Snapshot { cpu, cores, mem: parse_meminfo(&read("/proc/meminfo")), rx: rx_rate, tx: tx_rate, procs, load: [*load.first().unwrap_or(&0.0), *load.get(1).unwrap_or(&0.0), *load.get(2).unwrap_or(&0.0)], uptime }
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
        let users = parse_passwd("root:x:0:0:root:/root:/bin/sh\nuser:x:1000:100::/home/m:/bin/fish\n");
        assert_eq!((users[&0].as_str(), users[&1000].as_str()), ("root", "user"));
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
    fn times_and_rates_read_well() {
        assert_eq!(format_uptime(90061), "1 d 1 h");
        assert_eq!(format_uptime(7500), "2 h 05 min");
        assert_eq!(format_uptime(420), "7 min");
    }
}
