//! Background watcher for running Roblox clients.
//!
//! Once a second it samples every RobloxPlayerBeta process for CPU and memory use, and
//! applies RAM and CPU limits. A cap chosen for a client on the Home tab overrides the global
//! RAM limit for that client. The window reads the latest results from here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::winapi;

/// Global limits applied to every Roblox client. 0 means off.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Limits {
    pub ram_mb: u32,
    pub cpu_percent: u32,
}

/// One running Roblox client, as of the last sample.
#[derive(Clone, Debug)]
pub struct ProcInfo {
    pub pid: u32,
    /// Share of the whole machine's CPU (all cores), like Task Manager's process view.
    pub cpu_percent: f32,
    pub memory_mb: f32,
    pub ram_limited: bool,
    pub cpu_limited: bool,
}

#[derive(Clone)]
pub struct Monitor {
    pub limits: Arc<Mutex<Limits>>,
    /// RAM caps chosen for specific clients, in MB. They override the global RAM limit.
    pub ram_overrides: Arc<Mutex<HashMap<u32, u32>>>,
    pub procs: Arc<Mutex<Vec<ProcInfo>>>,
}

impl Monitor {
    /// Starts the background sampler. It runs for the life of the app.
    pub fn start(limits: Limits) -> Self {
        let monitor = Monitor {
            limits: Arc::new(Mutex::new(limits)),
            ram_overrides: Arc::new(Mutex::new(HashMap::new())),
            procs: Arc::new(Mutex::new(Vec::new())),
        };
        let shared_limits = monitor.limits.clone();
        let overrides = monitor.ram_overrides.clone();
        let procs = monitor.procs.clone();
        thread::spawn(move || run(shared_limits, overrides, procs));
        monitor
    }
}

/// What the monitor remembers about one client between samples.
struct Known {
    last: Option<(Instant, u64)>,
    /// The (RAM MB, CPU %) values last applied to this client.
    applied: Option<(u32, u32)>,
    ram_limited: bool,
    cpu_limited: bool,
}

fn run(
    limits: Arc<Mutex<Limits>>,
    overrides: Arc<Mutex<HashMap<u32, u32>>>,
    procs: Arc<Mutex<Vec<ProcInfo>>>,
) {
    let cores = winapi::cores().max(1) as f32;
    let mut known: HashMap<u32, Known> = HashMap::new();

    loop {
        let current = limits.lock().map(|l| *l).unwrap_or_default();
        let caps: HashMap<u32, u32> = overrides.lock().map(|m| m.clone()).unwrap_or_default();
        let pids = winapi::roblox_process_ids();
        let now = Instant::now();

        // Forget clients that have closed, so a new one with the same ID starts fresh.
        known.retain(|pid, _| pids.contains(pid));
        if let Ok(mut m) = overrides.lock() {
            m.retain(|pid, _| pids.contains(pid));
        }

        let mut snapshot = Vec::new();
        for pid in pids {
            let entry = known.entry(pid).or_insert(Known {
                last: None,
                applied: None,
                ram_limited: false,
                cpu_limited: false,
            });

            // A cap chosen for this client wins over the global RAM limit.
            let ram_target = caps.get(&pid).copied().unwrap_or(current.ram_mb);
            let cpu_target = current.cpu_percent;
            if entry.applied != Some((ram_target, cpu_target)) {
                // A RAM value of 0 removes the cap. A CPU job can't be removed, so a CPU cap stays once set.
                let ram_ok = winapi::limit_ram(pid, ram_target).is_ok();
                entry.ram_limited = ram_target > 0 && ram_ok;
                if cpu_target > 0 && !entry.cpu_limited {
                    entry.cpu_limited = winapi::limit_cpu(pid, cpu_target).is_ok();
                }
                entry.applied = Some((ram_target, cpu_target));
            }

            let Some(sample) = winapi::sample(pid) else {
                continue;
            };

            // CPU time is in 100 ns units. Compare it with the wall-clock time since the last sample.
            let cpu_percent = match entry.last {
                Some((then, cpu_before)) => {
                    let wall_100ns = now.duration_since(then).as_secs_f32() * 1e7;
                    if wall_100ns > 0.0 {
                        let used = sample.cpu_100ns.saturating_sub(cpu_before) as f32;
                        (used / wall_100ns / cores * 100.0).min(100.0)
                    } else {
                        0.0
                    }
                }
                None => 0.0,
            };
            entry.last = Some((now, sample.cpu_100ns));

            snapshot.push(ProcInfo {
                pid,
                cpu_percent,
                memory_mb: sample.working_set as f32 / 1_048_576.0,
                ram_limited: entry.ram_limited,
                cpu_limited: entry.cpu_limited,
            });
        }

        if let Ok(mut shared) = procs.lock() {
            *shared = snapshot;
        }
        thread::sleep(Duration::from_secs(1));
    }
}
