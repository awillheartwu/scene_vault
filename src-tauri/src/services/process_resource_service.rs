use chrono::{SecondsFormat, Utc};

use crate::models::resource::{ProcessResourceGroup, ProcessResourceStatus};

pub fn sample() -> ProcessResourceStatus {
    platform::sample()
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub fn sample() -> ProcessResourceStatus {
        ProcessResourceStatus {
            captured_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            approximate: true,
            logical_processors: std::thread::available_parallelism()
                .map(|value| value.get() as u32)
                .unwrap_or(1),
            groups: vec![ProcessResourceGroup {
                role: "rust".to_owned(),
                process_count: 1,
                pids: vec![std::process::id()],
                cpu_percent: None,
                working_set_bytes: 0,
                peak_working_set_bytes: 0,
                private_bytes: None,
            }],
            total_cpu_percent: None,
            total_working_set_bytes: 0,
            total_private_bytes: None,
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::{
        collections::{BTreeMap, HashMap, HashSet},
        mem::{size_of, zeroed},
        sync::{Mutex, OnceLock},
        time::Instant,
    };

    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
            ProcessStatus::{
                GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
            },
            Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
        },
    };

    use super::*;

    #[derive(Clone)]
    struct ProcessEntry {
        pid: u32,
        parent_pid: u32,
        name: String,
    }

    #[derive(Clone, Copy)]
    struct CpuSample {
        creation_ticks: u64,
        cpu_ticks: u64,
        observed_at: Instant,
    }

    #[derive(Default)]
    struct Aggregate {
        pids: Vec<u32>,
        cpu: Vec<f64>,
        working_set: u64,
        peak_working_set: u64,
        private_bytes: u64,
        measured_count: usize,
    }

    static CPU_SAMPLES: OnceLock<Mutex<HashMap<u32, CpuSample>>> = OnceLock::new();

    pub fn sample() -> ProcessResourceStatus {
        let logical = std::thread::available_parallelism()
            .map(|value| value.get() as u32)
            .unwrap_or(1);
        let mut entries = enumerate_processes();
        let own_pid = std::process::id();
        if !entries.iter().any(|entry| entry.pid == own_pid) {
            entries.push(ProcessEntry {
                pid: own_pid,
                parent_pid: 0,
                name: "scene_vault.exe".to_owned(),
            });
        }
        let mut included = HashSet::from([own_pid]);
        loop {
            let previous = included.len();
            for entry in &entries {
                if included.contains(&entry.parent_pid) {
                    included.insert(entry.pid);
                }
            }
            if included.len() == previous {
                break;
            }
        }

        let now = Instant::now();
        let samples = CPU_SAMPLES.get_or_init(|| Mutex::new(HashMap::new()));
        let mut previous = samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut aggregates: BTreeMap<&str, Aggregate> = BTreeMap::new();
        for entry in entries.iter().filter(|entry| included.contains(&entry.pid)) {
            let role = classify(entry, own_pid);
            let aggregate = aggregates.entry(role).or_default();
            aggregate.pids.push(entry.pid);
            if let Some((memory, cpu)) = inspect_process(entry.pid, now, logical, &mut previous) {
                aggregate.working_set = aggregate.working_set.saturating_add(memory.0);
                aggregate.peak_working_set = aggregate.peak_working_set.saturating_add(memory.1);
                aggregate.private_bytes = aggregate.private_bytes.saturating_add(memory.2);
                aggregate.measured_count += 1;
                if let Some(cpu) = cpu {
                    aggregate.cpu.push(cpu);
                }
            }
        }
        previous.retain(|pid, _| included.contains(pid));

        let role_order = ["rust", "webview", "python", "helper"];
        let groups = role_order
            .into_iter()
            .filter_map(|role| aggregates.remove(role).map(|aggregate| (role, aggregate)))
            .map(|(role, mut aggregate)| {
                aggregate.pids.sort_unstable();
                let process_count = aggregate.pids.len();
                let private_complete = aggregate.measured_count == process_count;
                ProcessResourceGroup {
                    role: role.to_owned(),
                    process_count: process_count as u32,
                    pids: aggregate.pids,
                    cpu_percent: (!aggregate.cpu.is_empty()).then(|| aggregate.cpu.iter().sum()),
                    working_set_bytes: aggregate.working_set,
                    peak_working_set_bytes: aggregate.peak_working_set,
                    private_bytes: private_complete.then_some(aggregate.private_bytes),
                }
            })
            .collect::<Vec<_>>();
        let cpu_values = groups
            .iter()
            .filter_map(|group| group.cpu_percent)
            .collect::<Vec<_>>();
        let private_values = groups
            .iter()
            .filter_map(|group| group.private_bytes)
            .collect::<Vec<_>>();
        let approximate = groups.iter().any(|group| group.private_bytes.is_none());
        ProcessResourceStatus {
            captured_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            approximate,
            logical_processors: logical,
            total_cpu_percent: (!cpu_values.is_empty()).then(|| cpu_values.iter().sum()),
            total_working_set_bytes: groups.iter().map(|group| group.working_set_bytes).sum(),
            total_private_bytes: (!groups.is_empty() && private_values.len() == groups.len())
                .then(|| private_values.iter().sum()),
            groups,
        }
    }

    fn classify(entry: &ProcessEntry, own_pid: u32) -> &'static str {
        if entry.pid == own_pid {
            return "rust";
        }
        let name = entry.name.to_ascii_lowercase();
        if name == "msedgewebview2.exe" {
            "webview"
        } else if matches!(
            name.as_str(),
            "python.exe" | "pythonw.exe" | "ai-worker.exe"
        ) {
            "python"
        } else {
            "helper"
        }
    }

    fn enumerate_processes() -> Vec<ProcessEntry> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Vec::new();
            }
            let mut entry: PROCESSENTRY32W = zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut result = Vec::new();
            let mut ok = Process32FirstW(snapshot, &mut entry) != 0;
            while ok {
                let length = entry
                    .szExeFile
                    .iter()
                    .position(|value| *value == 0)
                    .unwrap_or(entry.szExeFile.len());
                result.push(ProcessEntry {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                    name: String::from_utf16_lossy(&entry.szExeFile[..length]),
                });
                ok = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            result
        }
    }

    fn filetime_ticks(value: FILETIME) -> u64 {
        ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
    }

    fn inspect_process(
        pid: u32,
        now: Instant,
        logical: u32,
        previous: &mut HashMap<u32, CpuSample>,
    ) -> Option<((u64, u64, u64), Option<f64>)> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut counters: PROCESS_MEMORY_COUNTERS_EX = zeroed();
            counters.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
            let memory_ok = GetProcessMemoryInfo(
                handle,
                &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
                counters.cb,
            ) != 0;
            let mut creation: FILETIME = zeroed();
            let mut exit: FILETIME = zeroed();
            let mut kernel: FILETIME = zeroed();
            let mut user: FILETIME = zeroed();
            let time_ok =
                GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) != 0;
            CloseHandle(handle);
            if !memory_ok {
                return None;
            }
            let cpu = if time_ok {
                let current = CpuSample {
                    creation_ticks: filetime_ticks(creation),
                    cpu_ticks: filetime_ticks(kernel).saturating_add(filetime_ticks(user)),
                    observed_at: now,
                };
                let value = previous.get(&pid).and_then(|old| {
                    if old.creation_ticks != current.creation_ticks {
                        return None;
                    }
                    let elapsed = current
                        .observed_at
                        .duration_since(old.observed_at)
                        .as_secs_f64();
                    (elapsed > 0.0).then(|| {
                        let process_seconds =
                            current.cpu_ticks.saturating_sub(old.cpu_ticks) as f64 / 10_000_000.0;
                        (process_seconds / elapsed / logical.max(1) as f64 * 100.0)
                            .clamp(0.0, 100.0)
                    })
                });
                previous.insert(pid, current);
                value
            } else {
                None
            };
            Some((
                (
                    counters.WorkingSetSize as u64,
                    counters.PeakWorkingSetSize as u64,
                    counters.PrivateUsage as u64,
                ),
                cpu,
            ))
        }
    }
}
