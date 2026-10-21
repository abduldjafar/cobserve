//! Reads this machine (view 0): `ps` for the processes, `vm_stat` and `sysctl` for memory, swap,
//! pressure and load, and the kernel's CPU tick counters — on Linux `/proc` for the last three.
//! Each read a few milliseconds of child processes, on the runtime's own time: never the UI's.

use crate::app::Event;
use crate::local::{self, Part, Sample};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::sync::mpsc;

/// The sysctl keys a sample takes, in one call.
const SYSCTL_KEYS: [&str; 8] = [
    "kern.hostname",
    "hw.memsize",
    "hw.ncpu",
    "vm.swapusage",
    "kern.memorystatus_vm_pressure_level",
    "kern.memorystatus_level",
    "vm.loadavg",
    "kern.boottime",
];

pub async fn run(every: Duration, tx: mpsc::UnboundedSender<Event>) {
    let mut ticker = tokio::time::interval(every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut parts = Parts::new();
    loop {
        ticker.tick().await;
        let mut sample = read().await;
        fill_parts(&mut sample, &mut parts).await;
        if tx.send(Event::Local(Box::new(sample))).is_err() {
            return;
        }
    }
}

/// What each browser helper was found to be, by pid and command: a part never changes, so a
/// process's command line is read once, not every poll.
type Parts = std::collections::HashMap<u32, (String, Part, Option<f64>)>;

/// The parts of a browser's helpers (Chrome's `--type=`) and when each started: one `ps` for
/// the processes not seen before, and none at all when there are none.
async fn fill_parts(sample: &mut Sample, parts: &mut Parts) {
    parts.retain(|pid, (command, _, _)| sample.processes.iter().any(|p| p.pid == *pid && p.command == *command));
    let new: Vec<String> = sample
        .processes
        .iter()
        .filter(|p| local::has_parts(p.program()) && !parts.contains_key(&p.pid))
        .map(|p| p.pid.to_string())
        .collect();
    if !new.is_empty()
        && let Ok(text) = output("ps", &["-o", "pid=,etime=,args=", "-p", &new.join(",")]).await
    {
        for (pid, age_s, args) in local::parse_args(&text) {
            if let Some(p) = sample.processes.iter().find(|p| p.pid == pid) {
                parts.insert(pid, (p.command.clone(), local::part_of(&args), age_s.map(|a| sample.at - a)));
            }
        }
    }
    for p in &mut sample.processes {
        if let Some((_, part, started)) = parts.get(&p.pid) {
            p.part = Some(*part);
            p.started = *started;
        }
    }
}

async fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("{program}: {e}"))?;
    // sysctl says which keys it does not know on stderr, and still prints the others.
    if !out.status.success() && out.stdout.is_empty() {
        return Err(format!("{program} exited with {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// One read of the machine. What cannot be read is left out; only no `ps` at all is an error.
pub async fn read() -> Sample {
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    let mut sample = Sample { at, ticks: ticks(), ..Sample::default() };
    match output("ps", &["-axo", "pid=,ppid=,user=,pcpu=,rss=,time=,comm="]).await {
        Ok(text) => sample.processes = local::parse_ps(&text),
        Err(e) => sample.error = Some(e),
    }
    if cfg!(target_os = "linux") {
        if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
            sample.memory = local::parse_meminfo(&text);
        }
        if let Ok(text) = std::fs::read_to_string("/proc/loadavg") {
            let n: Vec<f64> = text.split_whitespace().take(3).filter_map(|n| n.parse().ok()).collect();
            if let [a, b, c] = n[..] {
                sample.load = Some([a, b, c]);
            }
        }
        sample.cores = std::thread::available_parallelism().map_or(0, |n| n.get() as u32);
        sample.host = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default().trim().to_string();
        return sample;
    }
    let total = match output("sysctl", &SYSCTL_KEYS).await {
        Ok(text) => local::parse_sysctl(&text, &mut sample),
        Err(_) => None,
    };
    if let (Some(total), Ok(text)) = (total, output("vm_stat", &[]).await) {
        sample.memory = local::parse_vm_stat(&text, total);
    }
    sample
}

/// The kernel's CPU tick counters since boot: user, system, idle, nice.
#[cfg(target_os = "macos")]
fn ticks() -> Option<[u64; 4]> {
    // mach/host_info.h. Declared here rather than taken from `libc`, which marks the Mach calls
    // deprecated in favour of a crate this one call does not need.
    const HOST_CPU_LOAD_INFO: i32 = 3;
    const HOST_CPU_LOAD_INFO_COUNT: u32 = 4;
    unsafe extern "C" {
        fn mach_host_self() -> u32;
        fn host_statistics(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
    }
    // One port right for the life of the process: each `mach_host_self` hands out another.
    static HOST: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    // SAFETY: `mach_host_self` takes nothing and returns a port name.
    let host = *HOST.get_or_init(|| unsafe { mach_host_self() });
    let mut info = [0u32; 4];
    let mut count = HOST_CPU_LOAD_INFO_COUNT;
    // SAFETY: `info` is HOST_CPU_LOAD_INFO_COUNT naturals, the size `count` tells the kernel it
    // may write; both outlive the call.
    let status = unsafe { host_statistics(host, HOST_CPU_LOAD_INFO, info.as_mut_ptr().cast(), &mut count) };
    // CPU_STATE_USER, _SYSTEM, _IDLE, _NICE — already the order of a sample's ticks.
    (status == 0 && count == HOST_CPU_LOAD_INFO_COUNT).then(|| info.map(u64::from))
}

#[cfg(target_os = "linux")]
fn ticks() -> Option<[u64; 4]> {
    local::parse_proc_stat(&std::fs::read_to_string("/proc/stat").ok()?)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn ticks() -> Option<[u64; 4]> {
    None
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn this_machine_reads() {
        let first = super::read().await;
        assert!(first.error.is_none(), "{:?}", first.error);
        assert!(!first.processes.is_empty() && first.cores > 0);
        if cfg!(target_os = "macos") {
            let m = first.memory.expect("vm_stat");
            assert!(m.total > 0 && m.used() <= m.total, "{m:?}");
            assert!(first.ticks.is_some() && first.pressure.is_some());
        }
    }

    /// `cargo test live_browser -- --ignored --nocapture`: this machine's browsers part by part,
    /// as the drill-down would show them (names of parts, counts and sizes only).
    #[tokio::test]
    #[ignore]
    async fn live_browser() {
        let mut parts = super::Parts::new();
        let mut local = crate::local::Local::default();
        for _ in 0..2 {
            let mut sample = super::read().await;
            super::fill_parts(&mut sample, &mut parts).await;
            local.record(sample);
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        for b in &local.browsers {
            println!("{} · {} processes · {}", b.program, b.count, crate::fmt::bytes(b.rss));
            for s in &b.parts {
                println!("  {:<10} {:>4} {:>10}", crate::browser::part_word(s.part), s.count, crate::fmt::bytes(s.rss));
            }
            for h in b.heaviest.iter().take(3) {
                println!("  heaviest {} {:?} {} age {:?}", h.pid, h.part, crate::fmt::bytes(h.rss), h.started.map(|s| local.latest.as_ref().unwrap().at - s));
            }
        }
    }

    /// `cargo test live_local -- --ignored --nocapture`: two reads two seconds apart, and what
    /// view 0 would say of them — to set beside Activity Monitor.
    #[tokio::test]
    #[ignore]
    async fn live_local() {
        let mut local = crate::local::Local::default();
        local.record(super::read().await);
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        local.record(super::read().await);
        let sample = local.latest.as_ref().unwrap();
        println!("cpu {:?}", local.cpu);
        println!("memory {:?} used {:.2} GiB", sample.memory, sample.memory.map_or(0.0, |m| crate::fmt::gib(m.used())));
        println!("swap {:?} pressure {:?} load {:?}", sample.swap, sample.pressure, sample.load);
        for row in local.rows().iter().take(8).chain(local.rows().last()) {
            println!("{:<36} {:>6} {:>5.2} {:>10}", row.name, row.pid, row.cores, crate::fmt::bytes(row.rss));
        }
    }
}
