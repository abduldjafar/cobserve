//! This machine — the laptop cobserve runs on — as Activity Monitor sees it: its CPU, its memory
//! and the pressure on it, and the processes that hold them (view 0, and the band's LOCAL line).
//!
//! A `Sample` is what one read of the machine says (`sources/local.rs` reads it: `ps`, `vm_stat`,
//! `sysctl` and the kernel's CPU ticks); `Local` keeps the last two and works out from them what
//! the screen shows. Pure, like the rest of the model: samples in, numbers out.
//!
//! The arithmetic (DESIGN.md §13):
//!
//! - **CPU**, the machine: `busy = Δ(user + system + nice) / Δ(all ticks)` between two reads of
//!   the kernel's tick counters, of `cores` logical cores; `busy cores = busy × cores`.
//! - **CPU**, a process: `cores = Δ cpu time / Δ wall` between two reads of `ps` — the delta form
//!   of §5.2 — and `%cpu / 100` (ps's own decaying average) for a process seen once.
//! - **the rest**: `busy cores − Σ process cores`, never below 0 — the kernel (`kernel_task` is not
//!   in `ps`) and what started and ended between two reads, so the rows add up to the machine.
//! - **memory used** = app + wired + compressed, Activity Monitor's sum: `app = anonymous −
//!   purgeable`, `cached = file-backed + purgeable`; all of `hw.memsize`.
//! - **pressure** is the kernel's: its level (normal, warning, critical) is the severity, and
//!   `kern.memorystatus_level` the share it counts as free. A Mac that uses 90% of its memory is
//!   well when the pressure is normal, so memory is coloured by the pressure, never by the used share.

use crate::history::Series;
use crate::severity::Severity;

/// What one read of the machine says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sample {
    /// When it was read, Unix seconds.
    pub at: f64,
    pub host: String,
    /// Logical cores: the denominator of every CPU number.
    pub cores: u32,
    /// The kernel's tick counters since boot: user, system, idle, nice.
    pub ticks: Option<[u64; 4]>,
    pub memory: Option<Memory>,
    /// Swap used and its size, bytes.
    pub swap: Option<(u64, u64)>,
    pub pressure: Option<Pressure>,
    /// The load average over 1, 5 and 15 minutes.
    pub load: Option<[f64; 3]>,
    /// Seconds since boot.
    pub uptime_s: Option<u64>,
    pub processes: Vec<Process>,
    /// Why the machine could not be read, when it could not.
    pub error: Option<String>,
}

/// Memory as Activity Monitor splits it, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Memory {
    pub total: u64,
    pub app: u64,
    pub wired: u64,
    pub compressed: u64,
    pub cached: u64,
}

impl Memory {
    pub fn used(&self) -> u64 {
        self.app + self.wired + self.compressed
    }

    pub fn used_pct(&self) -> Option<f64> {
        (self.total > 0).then(|| self.used() as f64 / self.total as f64 * 100.0)
    }
}

/// The kernel's word on memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pressure {
    /// `kern.memorystatus_vm_pressure_level`: 1 normal, 2 warning, 4 critical.
    pub level: u8,
    /// `kern.memorystatus_level`: the share of memory the kernel counts as free, 0–100.
    pub free_pct: u8,
}

impl Pressure {
    pub fn severity(&self) -> Severity {
        match self.level {
            4.. => Severity::Crit,
            2 | 3 => Severity::Warn,
            _ => Severity::Ok,
        }
    }

    pub fn word(&self) -> &'static str {
        match self.level {
            4.. => "critical",
            2 | 3 => "warning",
            _ => "normal",
        }
    }
}

/// One process, as `ps` lists it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    /// ps's `%cpu`: a decaying average, used only for a process seen once.
    pub pcpu: f64,
    /// Resident memory, bytes.
    pub rss: u64,
    /// CPU time since it started, seconds.
    pub cpu_time_s: f64,
    /// The executable's path, as `ps` gives it.
    pub command: String,
}

impl Process {
    /// What Activity Monitor calls it: the executable's own name.
    pub fn name(&self) -> &str {
        let path = self.command.trim_end_matches('/');
        path.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or(path)
    }

    /// The program it belongs to: the outermost `.app` it runs from — every `Google Chrome Helper
    /// (Renderer)` is Google Chrome — else its own name.
    pub fn program(&self) -> &str {
        self.command
            .split('/')
            .find_map(|part| part.strip_suffix(".app"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.name())
    }
}

/// The machine's CPU between two reads.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Cpu {
    pub user_pct: f64,
    pub system_pct: f64,
    pub busy_pct: f64,
    pub busy_cores: f64,
}

/// A row of view 0: a process, or every process of one program.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    /// The process's id; for a program, its busiest process's.
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    /// How many processes the row holds: 1, or a program's count.
    pub count: usize,
    pub cores: f64,
    pub rss: u64,
    pub cpu_time_s: f64,
    pub command: String,
    /// The closing row: `the rest` of the CPU the rows above do not account for.
    pub rest: bool,
}

/// What view 0 is sorted by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Cpu,
    Memory,
}

/// The machine over time: the last two samples and what they say.
#[derive(Debug, Clone, Default)]
pub struct Local {
    pub latest: Option<Sample>,
    previous: Option<Sample>,
    pub cpu: Option<Cpu>,
    pub cpu_pct: Series,
    pub mem_pct: Series,
    pub sort: Sort,
    /// One row per program instead of per process.
    pub grouped: bool,
    /// The row under the cursor, and what it is — its pid, or its program — so the cursor stays
    /// on it while the rows re-sort under it every read.
    pub selected: usize,
    selected_key: Option<String>,
}

impl Local {
    pub fn record(&mut self, sample: Sample) {
        if sample.error.is_some() && sample.processes.is_empty() {
            self.previous = None;
            self.cpu = None;
            self.latest = Some(sample);
            return;
        }
        self.previous = self.latest.take().filter(|p| p.error.is_none() && p.at < sample.at);
        self.cpu = match (self.previous.as_ref().and_then(|p| p.ticks), sample.ticks) {
            (Some(before), Some(now)) => cpu(before, now, sample.cores),
            _ => None,
        };
        if let Some(cpu) = self.cpu {
            self.cpu_pct.push(sample.at, cpu.busy_pct);
        }
        if let Some(pct) = sample.memory.and_then(|m| m.used_pct()) {
            self.mem_pct.push(sample.at, pct);
        }
        self.latest = Some(sample);
        let rows = self.rows();
        let found = self.selected_key.as_ref().and_then(|k| rows.iter().position(|r| self.key(r) == *k));
        self.selected = found.unwrap_or(self.selected).min(rows.len().saturating_sub(1));
    }

    fn key(&self, row: &Row) -> String {
        if self.grouped || row.rest { row.name.clone() } else { row.pid.to_string() }
    }

    /// Put the cursor on row `index` of `rows()`, or as near it as there is.
    pub fn select(&mut self, index: usize) {
        let rows = self.rows();
        self.selected = index.min(rows.len().saturating_sub(1));
        self.selected_key = rows.get(self.selected).map(|r| self.key(r));
    }

    pub fn select_by(&mut self, delta: isize) {
        self.select(self.selected.saturating_add_signed(delta));
    }

    /// Whether there is a machine to show: read at least once, and not refused.
    pub fn shown(&self) -> bool {
        self.latest.as_ref().is_some_and(|s| s.error.is_none() || !s.processes.is_empty())
    }

    /// Each process's cores between the last two reads: the delta form, else ps's average.
    pub fn process_cores(&self) -> Vec<(&Process, f64)> {
        let Some(latest) = &self.latest else { return Vec::new() };
        let before: std::collections::HashMap<u32, &Process> =
            self.previous.iter().flat_map(|p| p.processes.iter()).map(|p| (p.pid, p)).collect();
        let wall = self.previous.as_ref().map_or(0.0, |p| latest.at - p.at);
        latest
            .processes
            .iter()
            .map(|p| {
                let cores = match before.get(&p.pid) {
                    // The same pid, the same command, CPU time that went forward: the delta.
                    Some(old) if wall > 0.0 && old.command == p.command && p.cpu_time_s >= old.cpu_time_s => {
                        (p.cpu_time_s - old.cpu_time_s) / wall
                    }
                    _ => p.pcpu / 100.0,
                };
                (p, cores.max(0.0))
            })
            .collect()
    }

    /// The rows of view 0, sorted, and — when the machine's CPU is known — closed by `the rest`.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = Vec::new();
        let processes = self.process_cores();
        if self.grouped {
            let mut programs: Vec<(String, Vec<(&Process, f64)>)> = Vec::new();
            for (p, cores) in processes {
                match programs.iter_mut().find(|(name, _)| name == p.program()) {
                    Some((_, members)) => members.push((p, cores)),
                    None => programs.push((p.program().to_string(), vec![(p, cores)])),
                }
            }
            for (name, members) in programs {
                let busiest = members.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|m| m.0);
                let Some(busiest) = busiest else { continue };
                rows.push(Row {
                    name,
                    pid: busiest.pid,
                    ppid: busiest.ppid,
                    user: busiest.user.clone(),
                    count: members.len(),
                    cores: members.iter().map(|m| m.1).sum(),
                    rss: members.iter().map(|m| m.0.rss).sum(),
                    cpu_time_s: members.iter().map(|m| m.0.cpu_time_s).sum(),
                    command: busiest.command.clone(),
                    rest: false,
                });
            }
        } else {
            rows.extend(processes.into_iter().map(|(p, cores)| Row {
                name: p.name().to_string(),
                pid: p.pid,
                ppid: p.ppid,
                user: p.user.clone(),
                count: 1,
                cores,
                rss: p.rss,
                cpu_time_s: p.cpu_time_s,
                command: p.command.clone(),
                rest: false,
            }));
        }
        match self.sort {
            Sort::Cpu => rows.sort_by(|a, b| b.cores.total_cmp(&a.cores).then(b.rss.cmp(&a.rss)).then(a.pid.cmp(&b.pid))),
            Sort::Memory => rows.sort_by(|a, b| b.rss.cmp(&a.rss).then(b.cores.total_cmp(&a.cores)).then(a.pid.cmp(&b.pid))),
        }
        if let Some(cpu) = self.cpu {
            let counted: f64 = rows.iter().map(|r| r.cores).sum();
            rows.push(Row {
                name: "the rest".into(),
                pid: 0,
                ppid: 0,
                user: String::new(),
                count: 0,
                cores: (cpu.busy_cores - counted).max(0.0),
                rss: 0,
                cpu_time_s: 0.0,
                command: "the kernel, and what started and ended between two reads".into(),
                rest: true,
            });
        }
        rows
    }

    /// The busiest process now, by cores.
    pub fn top(&self) -> Option<(&Process, f64)> {
        self.process_cores().into_iter().max_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// How the machine is: the worse of its CPU (75 and 90, as a node's) and its memory pressure.
    pub fn severity(&self) -> Severity {
        let cpu = crate::severity::node(self.cpu.map(|c| c.busy_pct));
        let pressure = self.latest.as_ref().and_then(|s| s.pressure).map_or(Severity::None, |p| p.severity());
        if cpu == Severity::Crit || pressure == Severity::Crit {
            Severity::Crit
        } else if cpu == Severity::Warn || pressure == Severity::Warn {
            Severity::Warn
        } else {
            Severity::Ok
        }
    }
}

/// The machine's CPU between two reads of the tick counters.
pub fn cpu(before: [u64; 4], now: [u64; 4], cores: u32) -> Option<Cpu> {
    let d: Vec<f64> = (0..4).map(|i| now[i].saturating_sub(before[i]) as f64).collect();
    let all: f64 = d.iter().sum();
    if all <= 0.0 || now.iter().zip(before).any(|(n, b)| *n < b) {
        return None;
    }
    let (user, system, nice) = (d[0] / all * 100.0, d[1] / all * 100.0, d[3] / all * 100.0);
    let busy = user + system + nice;
    Some(Cpu { user_pct: user + nice, system_pct: system, busy_pct: busy, busy_cores: busy / 100.0 * f64::from(cores) })
}

// ---------------------------------------------------------------------------
// Reading what the commands print
// ---------------------------------------------------------------------------

/// `ps -axo pid=,ppid=,user=,pcpu=,rss=,time=,comm=`: one process a line, the command last, as
/// it may hold spaces. A line that does not read is skipped.
pub fn parse_ps(text: &str) -> Vec<Process> {
    text.lines()
        .filter_map(|line| {
            let mut rest = line.trim_start();
            let mut field = || {
                let end = rest.find(char::is_whitespace)?;
                let (word, tail) = rest.split_at(end);
                rest = tail.trim_start();
                Some(word)
            };
            let pid = field()?.parse().ok()?;
            let ppid = field()?.parse().ok()?;
            let user = field()?.to_string();
            let pcpu = field()?.replace(',', ".").parse().ok()?;
            let rss_kib: u64 = field()?.parse().ok()?;
            let cpu_time_s = cpu_time(field()?)?;
            let command = rest.trim_end().to_string();
            (!command.is_empty()).then_some(Process { pid, ppid, user, pcpu, rss: rss_kib * 1024, cpu_time_s, command })
        })
        .collect()
}

/// ps's `time`: `2:34.27` and `3605:00.08` on macOS, `[[dd-]hh:]mm:ss` on Linux.
pub fn cpu_time(text: &str) -> Option<f64> {
    let (days, clock) = match text.split_once('-') {
        Some((d, rest)) => (d.parse::<f64>().ok()?, rest),
        None => (0.0, text),
    };
    let mut seconds = 0.0;
    for part in clock.split(':') {
        seconds = seconds * 60.0 + part.replace(',', ".").parse::<f64>().ok()?;
    }
    Some(days * 86_400.0 + seconds)
}

/// `vm_stat`: memory as Activity Monitor splits it, of `total` bytes.
pub fn parse_vm_stat(text: &str, total: u64) -> Option<Memory> {
    let page: u64 = text
        .lines()
        .next()?
        .split("page size of ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let pages = |name: &str| -> Option<u64> {
        let line = text.lines().find(|l| l.trim_start_matches('"').starts_with(name))?;
        line.rsplit(':').next()?.trim().trim_end_matches('.').parse::<u64>().ok().map(|n| n * page)
    };
    let anonymous = pages("Anonymous pages")?;
    let purgeable = pages("Pages purgeable").unwrap_or(0);
    Some(Memory {
        total,
        app: anonymous.saturating_sub(purgeable),
        wired: pages("Pages wired down")?,
        compressed: pages("Pages occupied by compressor").unwrap_or(0),
        cached: pages("File-backed pages").unwrap_or(0) + purgeable,
    })
}

/// `sysctl` with its keys (`key: value` a line): what the sample takes from it. A key the kernel
/// does not have is left out of the answer, not an error.
pub fn parse_sysctl(text: &str, sample: &mut Sample) -> Option<u64> {
    let value = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(':').map(str::trim));
    if let Some(host) = value("kern.hostname") {
        sample.host = host.to_string();
    }
    if let Some(cores) = value("hw.ncpu").and_then(|v| v.parse().ok()) {
        sample.cores = cores;
    }
    // `total = 3072.00M  used = 2280.94M  free = 791.06M  (encrypted)`
    if let Some(swap) = value("vm.swapusage") {
        let size = |name: &str| -> Option<u64> {
            let after = swap.split(&format!("{name} = ")).nth(1)?;
            let word = after.split_whitespace().next()?;
            let (number, unit) = word.split_at(word.len().checked_sub(1)?);
            let scale = match unit {
                "K" => 1u64 << 10,
                "M" => 1 << 20,
                "G" => 1 << 30,
                _ => return None,
            };
            Some((number.parse::<f64>().ok()? * scale as f64) as u64)
        };
        if let (Some(used), Some(total)) = (size("used"), size("total")) {
            sample.swap = Some((used, total));
        }
    }
    if let (Some(level), Some(free)) = (
        value("kern.memorystatus_vm_pressure_level").and_then(|v| v.parse().ok()),
        value("kern.memorystatus_level").and_then(|v| v.parse().ok()),
    ) {
        sample.pressure = Some(Pressure { level, free_pct: free });
    }
    // `{ 8.06 4.21 3.46 }`
    if let Some(load) = value("vm.loadavg") {
        let numbers: Vec<f64> = load.trim_matches(['{', '}', ' ']).split_whitespace().filter_map(|n| n.parse().ok()).collect();
        if let [one, five, fifteen] = numbers[..] {
            sample.load = Some([one, five, fifteen]);
        }
    }
    // `{ sec = 1788653667, usec = 60213 } Sun Sep  6 07:14:27 2026`
    if let Some(boot) = value("kern.boottime") {
        let sec = boot.split("sec = ").nth(1).and_then(|s| s.split(',').next()).and_then(|s| s.trim().parse::<f64>().ok());
        if let Some(sec) = sec {
            sample.uptime_s = Some((sample.at - sec).max(0.0) as u64);
        }
    }
    value("hw.memsize").and_then(|v| v.parse().ok())
}

/// Linux's `/proc/meminfo`: used is what is not available, there being no compressor to split.
pub fn parse_meminfo(text: &str) -> Option<Memory> {
    let kib = |name: &str| -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with(name))?;
        line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|n| n * 1024)
    };
    let total = kib("MemTotal:")?;
    let available = kib("MemAvailable:")?;
    let cached = kib("Cached:").unwrap_or(0);
    Some(Memory { total, app: total.saturating_sub(available), wired: 0, compressed: 0, cached })
}

/// Linux's `/proc/stat` first line: user, system, idle, nice ticks.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_proc_stat(text: &str) -> Option<[u64; 4]> {
    let fields: Vec<u64> = text.lines().next()?.strip_prefix("cpu ")?.split_whitespace().filter_map(|n| n.parse().ok()).collect();
    // user nice system idle iowait irq softirq steal
    let busy_system = fields.get(2)? + fields.get(5).unwrap_or(&0) + fields.get(6).unwrap_or(&0);
    let idle = fields.get(3)? + fields.get(4).unwrap_or(&0);
    Some([*fields.first()?, busy_system, idle, *fields.get(1)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    const VM_STAT: &str = "Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                                     4430.
Pages active:                                 513965.
Pages wired down:                             186679.
Pages purgeable:                                5735.
\"Translation faults\":                    13715029590.
File-backed pages:                            307277.
Anonymous pages:                              719537.
Pages stored in compressor:                  2276914.
Pages occupied by compressor:                 822517.
";

    #[test]
    fn memory_is_split_as_activity_monitor_splits_it() {
        let total = 34_359_738_368;
        let m = parse_vm_stat(VM_STAT, total).unwrap();
        let page = 16384;
        assert_eq!(m.app, (719_537 - 5735) * page);
        assert_eq!(m.wired, 186_679 * page);
        assert_eq!(m.compressed, 822_517 * page);
        assert_eq!(m.cached, (307_277 + 5735) * page);
        assert_eq!(m.used(), m.app + m.wired + m.compressed);
        let pct = m.used_pct().unwrap();
        assert!((pct - 82.2).abs() < 0.1, "{pct}");
    }

    #[test]
    fn sysctl_says_cores_swap_pressure_and_load() {
        let text = "kern.hostname: mac.local
hw.memsize: 34359738368
hw.ncpu: 10
vm.swapusage: total = 3072.00M  used = 2280.94M  free = 791.06M  (encrypted)
kern.memorystatus_vm_pressure_level: 2
kern.memorystatus_level: 50
vm.loadavg: { 8.06 4.21 3.46 }
kern.boottime: { sec = 1788653667, usec = 60213 } Sun Sep  6 07:14:27 2026
";
        let mut sample = Sample { at: 1_788_653_667.0 + 3600.0, ..Sample::default() };
        assert_eq!(parse_sysctl(text, &mut sample), Some(34_359_738_368));
        assert_eq!(sample.host, "mac.local");
        assert_eq!(sample.cores, 10);
        assert_eq!(sample.swap, Some(((2280.94 * 1048576.0) as u64, 3072 << 20)));
        assert_eq!(sample.pressure, Some(Pressure { level: 2, free_pct: 50 }));
        assert_eq!(sample.pressure.unwrap().severity(), Severity::Warn);
        assert_eq!(sample.load, Some([8.06, 4.21, 3.46]));
        assert_eq!(sample.uptime_s, Some(3600));
        // A kernel without the pressure keys: no pressure, not a wrong one.
        let mut bare = Sample::default();
        parse_sysctl("hw.ncpu: 4\n", &mut bare);
        assert_eq!((bare.cores, bare.pressure), (4, None));
    }

    #[test]
    fn ps_lines_keep_the_spaces_in_a_command() {
        let text = "28839 28809 macbook           29.8 571824   2:34.27 opencode
  164     1 _windowserver      4.3 140048 3605:00.08 /System/Library/PrivateFrameworks/SkyLight.framework/Resources/WindowServer
15564 14897 macbook 1,5 853000 1:02.00 /Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)
not a line
";
        let ps = parse_ps(text);
        assert_eq!(ps.len(), 3);
        assert_eq!((ps[0].pid, ps[0].ppid, ps[0].name(), ps[0].rss), (28839, 28809, "opencode", 571_824 * 1024));
        assert!((ps[0].cpu_time_s - 154.27).abs() < 1e-9);
        assert!((ps[1].cpu_time_s - 216_300.08).abs() < 1e-6);
        assert_eq!(ps[1].name(), "WindowServer");
        assert_eq!(ps[1].program(), "WindowServer");
        assert_eq!(ps[2].name(), "Google Chrome Helper (Renderer)");
        assert_eq!(ps[2].program(), "Google Chrome", "the outermost .app");
        assert!((ps[2].pcpu - 1.5).abs() < 1e-9, "a decimal comma reads too");
        assert_eq!(cpu_time("1-02:03:04"), Some(86_400.0 + 7384.0));
        assert_eq!(cpu_time("x"), None);
    }

    #[test]
    fn the_machine_cpu_is_the_ticks_that_were_not_idle() {
        let cpu = cpu([100, 50, 800, 0], [160, 70, 920, 0], 10).unwrap();
        assert!((cpu.busy_pct - 40.0).abs() < 1e-9);
        assert!((cpu.user_pct - 30.0).abs() < 1e-9 && (cpu.system_pct - 10.0).abs() < 1e-9);
        assert!((cpu.busy_cores - 4.0).abs() < 1e-9);
        assert_eq!(super::cpu([1, 1, 1, 1], [1, 1, 1, 1], 10), None, "no ticks went by");
        assert_eq!(super::cpu([5, 1, 1, 1], [1, 1, 9, 1], 10), None, "a counter went back: a reboot");
    }

    fn process(pid: u32, command: &str, cpu_time_s: f64, rss_mib: u64, pcpu: f64) -> Process {
        Process { pid, ppid: 1, user: "me".into(), pcpu, rss: rss_mib << 20, cpu_time_s, command: command.into() }
    }

    fn sample(at: f64, ticks: [u64; 4], processes: Vec<Process>) -> Sample {
        Sample { at, cores: 10, ticks: Some(ticks), processes, ..Sample::default() }
    }

    #[test]
    fn processes_take_the_delta_and_the_rest_closes_the_machine() {
        let mut local = Local::default();
        let chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
        let helper = "/Applications/Google Chrome.app/Contents/Frameworks/x.framework/Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper";
        local.record(sample(100.0, [0, 0, 0, 0], vec![process(1, "/bin/opencode", 10.0, 800, 47.0), process(2, chrome, 5.0, 900, 3.0)]));
        // Seen once: ps's own average, and no machine CPU yet, so no closing row.
        let rows = local.rows();
        assert_eq!(rows.len(), 2);
        assert!((rows[0].cores - 0.47).abs() < 1e-9 && rows[0].name == "opencode");

        // Two seconds later: opencode used 1 s of CPU (0.5 cores), Chrome 0.2 s, a helper is new.
        let ticks = [600, 200, 1200, 0]; // 40% busy of 10 cores = 4 cores
        local.record(sample(102.0, ticks, vec![
            process(1, "/bin/opencode", 11.0, 800, 47.0),
            process(2, chrome, 5.2, 900, 3.0),
            process(3, helper, 1.0, 400, 20.0),
        ]));
        let rows = local.rows();
        let cores: Vec<(String, f64)> = rows.iter().map(|r| (r.name.clone(), (r.cores * 100.0).round() / 100.0)).collect();
        assert_eq!(cores, [("opencode".into(), 0.5), ("Google Chrome Helper".into(), 0.2), ("Google Chrome".into(), 0.1), ("the rest".into(), 3.2)]);
        let total: f64 = rows.iter().map(|r| r.cores).sum();
        assert!((total - local.cpu.unwrap().busy_cores).abs() < 1e-9, "the rows add up to the machine");

        local.grouped = true;
        let rows = local.rows();
        assert_eq!(rows[0].name, "opencode");
        assert_eq!((rows[1].name.as_str(), rows[1].count, rows[1].rss), ("Google Chrome", 2, 1300 << 20));
        assert!((rows[1].cores - 0.3).abs() < 1e-9);
        assert_eq!(rows[1].pid, 3, "a program's row points at its busiest process");

        local.sort = Sort::Memory;
        assert_eq!(local.rows()[0].name, "Google Chrome");
    }

    #[test]
    fn the_cursor_stays_on_its_process_while_the_rows_re_sort() {
        let mut local = Local::default();
        local.record(sample(100.0, [0, 0, 1, 0], vec![process(1, "/bin/a", 0.0, 1, 50.0), process(2, "/bin/b", 0.0, 1, 10.0)]));
        local.select(1);
        assert_eq!(local.rows()[local.selected].name, "b");
        local.record(sample(102.0, [1, 0, 2, 0], vec![process(1, "/bin/a", 0.0, 1, 0.0), process(2, "/bin/b", 2.0, 1, 10.0)]));
        assert_eq!(local.rows()[local.selected].name, "b", "b is busier now and first; the cursor went with it");
        assert_eq!(local.selected, 0);
    }

    #[test]
    fn a_reused_pid_is_not_a_delta() {
        let mut local = Local::default();
        local.record(sample(100.0, [0, 0, 1, 0], vec![process(7, "/bin/a", 50.0, 1, 0.0)]));
        local.record(sample(102.0, [1, 0, 2, 0], vec![process(7, "/bin/b", 1.0, 1, 12.0)]));
        assert!((local.rows()[0].cores - 0.12).abs() < 1e-9);
    }

    #[test]
    fn memory_takes_its_colour_from_the_pressure() {
        let mut local = Local::default();
        let mut s = sample(100.0, [0, 0, 100, 0], Vec::new());
        s.memory = Some(Memory { total: 100, app: 60, wired: 20, compressed: 15, cached: 5 });
        s.pressure = Some(Pressure { level: 1, free_pct: 50 });
        local.record(s.clone());
        assert_eq!(local.severity(), Severity::Ok, "95% used and normal pressure is a well Mac");
        s.at = 102.0;
        s.ticks = Some([180, 0, 120, 0]); // 90% busy
        local.record(s);
        assert_eq!(local.severity(), Severity::Crit);
    }

    #[test]
    fn linux_reads_from_proc() {
        let m = parse_meminfo("MemTotal:       16000000 kB\nMemFree: 1 kB\nMemAvailable:    4000000 kB\nCached: 2000000 kB\n").unwrap();
        assert_eq!((m.total, m.used()), (16_000_000 * 1024, 12_000_000 * 1024));
        assert_eq!(parse_proc_stat("cpu  10 2 30 400 5 6 7 0\ncpu0 1 1 1 1\n"), Some([10, 43, 405, 2]));
    }
}
