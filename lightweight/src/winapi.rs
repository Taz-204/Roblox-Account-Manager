//! Roblox window and process tuning, done in-process with Win32 calls.
//!
//! These used to start RobloxProcessLookup.exe for every action. Calling Windows directly
//! avoids starting a .NET process each time.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::mem::size_of;

    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOBOBJECTINFOCLASS, SetInformationJobObject,
    };
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{
        PROCESS_QUERY_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE, PROCESS_VM_READ,
        QUOTA_LIMITS_HARDWS_MAX_ENABLE, SetProcessWorkingSetSizeEx,
    };
    use windows::core::PCWSTR;

    use windows::Win32::Foundation::{BOOL, CloseHandle, FILETIME, HANDLE, HWND, LPARAM, RECT};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        BELOW_NORMAL_PRIORITY_CLASS, GetProcessTimes, NORMAL_PRIORITY_CLASS, OpenProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, SetPriorityClass,
        SetProcessAffinityMask,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GetClientRect, GetWindow, GetWindowRect, GetWindowThreadProcessId,
        IsWindowVisible, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
    };

    const ROBLOX_EXE: &str = "RobloxPlayerBeta.exe";
    /// Same cap as the C# helper, so the affinity mask fits in 64 bits.
    const MAX_CORES: usize = 62;

    /// Process IDs of every running Roblox game process.
    fn roblox_pids() -> Result<Vec<u32>, String> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|e| e.to_string())?;
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut pids = Vec::new();
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case(ROBLOX_EXE) {
                pids.push(entry.th32ProcessID);
            }
            more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
        }
        unsafe {
            let _ = CloseHandle(snapshot);
        }
        Ok(pids)
    }

    struct WindowSearch {
        pid: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn take_main_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut WindowSearch) };
        let mut owner_pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner_pid)) };
        let is_visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        // A top-level window has no owner. Roblox's main game window is the first one we see.
        let has_owner = unsafe { GetWindow(hwnd, GW_OWNER) }
            .map(|owner| !owner.0.is_null())
            .unwrap_or(false);
        if owner_pid == search.pid && is_visible && !has_owner {
            search.found = Some(hwnd);
            return BOOL(0); // stop enumerating
        }
        BOOL(1)
    }

    /// Each Roblox process with its main game window, if it has one.
    fn roblox_windows() -> Result<Vec<(u32, HWND)>, String> {
        let mut result = Vec::new();
        for pid in roblox_pids()? {
            let mut search = WindowSearch { pid, found: None };
            let _ = unsafe {
                EnumWindows(
                    Some(take_main_window),
                    LPARAM(&mut search as *mut WindowSearch as isize),
                )
            };
            if let Some(hwnd) = search.found {
                result.push((pid, hwnd));
            }
        }
        Ok(result)
    }

    /// Sets each Roblox window's client area to width x height. Returns the size applied to each window.
    pub fn resize_windows(width: u32, height: u32) -> Result<Vec<String>, String> {
        if !(1..=4000).contains(&width) || !(1..=4000).contains(&height) {
            return Err("width and height must be from 1 to 4000".into());
        }
        let mut sizes = Vec::new();
        for (_, hwnd) in roblox_windows()? {
            let mut outer = RECT::default();
            let mut client = RECT::default();
            let got_outer = unsafe { GetWindowRect(hwnd, &mut outer) }.is_ok();
            let got_client = unsafe { GetClientRect(hwnd, &mut client) }.is_ok();
            if !got_outer || !got_client {
                continue;
            }
            // The outer window includes borders and the title bar, so add them back on.
            let border_w = (outer.right - outer.left) - (client.right - client.left);
            let border_h = (outer.bottom - outer.top) - (client.bottom - client.top);
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    outer.left,
                    outer.top,
                    width as i32 + border_w,
                    height as i32 + border_h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            let mut after = RECT::default();
            if unsafe { GetClientRect(hwnd, &mut after) }.is_ok() {
                sizes.push(format!(
                    "{}x{}",
                    after.right - after.left,
                    after.bottom - after.top
                ));
            }
        }
        Ok(sizes)
    }

    /// Sets the priority of every running Roblox process (Normal, or Below Normal). Returns how many changed.
    pub fn set_priority(normal: bool) -> Result<usize, String> {
        let class = if normal {
            NORMAL_PRIORITY_CLASS
        } else {
            BELOW_NORMAL_PRIORITY_CLASS
        };
        let mut changed = 0;
        for pid in roblox_pids()? {
            if let Ok(handle) = unsafe { OpenProcess(PROCESS_SET_INFORMATION, false, pid) } {
                if unsafe { SetPriorityClass(handle, class) }.is_ok() {
                    changed += 1;
                }
                unsafe {
                    let _ = CloseHandle(handle);
                }
            }
        }
        Ok(changed)
    }

    /// Gives each Roblox window its own group of CPU cores, oldest window first.
    /// Returns how many windows were changed.
    pub fn spread_across_cores() -> Result<usize, String> {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(MAX_CORES);

        // (start time, process handle) for each Roblox window.
        let mut procs: Vec<(u64, HANDLE)> = Vec::new();
        for (pid, _) in roblox_windows()? {
            let access = PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION;
            let handle = match unsafe { OpenProcess(access, false, pid) } {
                Ok(handle) => handle,
                Err(_) => continue,
            };
            let mut created = FILETIME::default();
            let mut exited = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            let start = if unsafe {
                GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user)
            }
            .is_ok()
            {
                ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64
            } else {
                u64::MAX
            };
            procs.push((start, handle));
        }
        procs.sort_by_key(|p| p.0);

        let mut changed = 0;
        if !procs.is_empty() && cores > 0 {
            let group = (cores / procs.len()).max(1);
            for (i, (_, handle)) in procs.iter().enumerate() {
                let mut mask: u64 = 0;
                for c in 0..group {
                    mask |= 1u64 << ((i * group + c) % cores);
                }
                if unsafe { SetProcessAffinityMask(*handle, mask as usize) }.is_ok() {
                    changed += 1;
                }
            }
        }

        for (_, handle) in procs {
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
        Ok(changed)
    }
    /// Number of logical CPU cores.
    pub fn cores() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    }

    /// Process IDs of running Roblox clients. Empty if the list can't be read.
    pub fn roblox_process_ids() -> Vec<u32> {
        roblox_pids().unwrap_or_default()
    }

    fn filetime_to_u64(ft: FILETIME) -> u64 {
        ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
    }

    /// CPU time (100 ns units, user plus kernel) and working-set size for one process.
    pub fn sample(pid: u32) -> Option<ProcSample> {
        let handle =
            unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) }.ok()?;
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let times_ok =
            unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
                .is_ok();

        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        let cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        counters.cb = cb;
        let mem_ok = unsafe { GetProcessMemoryInfo(handle, &mut counters, cb) }.as_bool();

        unsafe {
            let _ = CloseHandle(handle);
        }
        if !times_ok {
            return None;
        }
        Some(ProcSample {
            cpu_100ns: filetime_to_u64(kernel) + filetime_to_u64(user),
            working_set: if mem_ok {
                counters.WorkingSetSize as u64
            } else {
                0
            },
        })
    }

    /// Caps the process's physical memory (working set) at `megabytes`. Extra memory is paged out
    /// rather than the client being stopped.
    pub fn limit_ram(pid: u32, megabytes: u32) -> Result<(), String> {
        let access = PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION;
        let handle = unsafe { OpenProcess(access, false, pid) }.map_err(|e| e.to_string())?;
        let max = megabytes as usize * 1024 * 1024;
        let min = (1024 * 1024usize).min(max);
        let result =
            unsafe { SetProcessWorkingSetSizeEx(handle, min, max, QUOTA_LIMITS_HARDWS_MAX_ENABLE) }
                .map_err(|e| e.to_string());
        unsafe {
            let _ = CloseHandle(handle);
        }
        result
    }

    /// Hard CPU cap for the process, as a percent of all cores. It runs inside its own Job Object.
    pub fn limit_cpu(pid: u32, percent: u32) -> Result<(), String> {
        if !(1..=100).contains(&percent) {
            return Err("CPU limit must be 1 to 100 percent".into());
        }
        let access = PROCESS_SET_QUOTA | PROCESS_TERMINATE;
        let process = unsafe { OpenProcess(access, false, pid) }.map_err(|e| e.to_string())?;

        let result = (|| -> Result<(), String> {
            let job =
                unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(|e| e.to_string())?;
            // CpuRate is in 1/100 of a percent, so 100% is 10000.
            let info = CpuRateInfo {
                control_flags: JOB_CPU_ENABLE | JOB_CPU_HARD_CAP,
                cpu_rate: percent * 100,
            };
            let set = unsafe {
                SetInformationJobObject(
                    job,
                    JOBOBJECTINFOCLASS(JOB_CPU_RATE_CONTROL_CLASS),
                    &info as *const CpuRateInfo as *const c_void,
                    size_of::<CpuRateInfo>() as u32,
                )
            };
            let assigned = set.and_then(|_| unsafe { AssignProcessToJobObject(job, process) });
            // The job keeps the limit while the process runs, so this handle can be closed.
            unsafe {
                let _ = CloseHandle(job);
            }
            assigned.map_err(|e| e.to_string())
        })();

        unsafe {
            let _ = CloseHandle(process);
        }
        result
    }

    /// Layout of JOBOBJECT_CPU_RATE_CONTROL_INFORMATION: a flags word and a rate.
    #[repr(C)]
    struct CpuRateInfo {
        control_flags: u32,
        cpu_rate: u32,
    }
    const JOB_CPU_ENABLE: u32 = 0x1;
    const JOB_CPU_HARD_CAP: u32 = 0x4;
    const JOB_CPU_RATE_CONTROL_CLASS: i32 = 15;
}

#[cfg(not(windows))]
mod imp {
    const ONLY_WINDOWS: &str = "Roblox window tools only work on Windows.";

    pub fn resize_windows(_width: u32, _height: u32) -> Result<Vec<String>, String> {
        Err(ONLY_WINDOWS.into())
    }

    pub fn set_priority(_normal: bool) -> Result<usize, String> {
        Err(ONLY_WINDOWS.into())
    }

    pub fn spread_across_cores() -> Result<usize, String> {
        Err(ONLY_WINDOWS.into())
    }

    pub fn cores() -> usize {
        1
    }

    pub fn roblox_process_ids() -> Vec<u32> {
        Vec::new()
    }

    pub fn sample(_pid: u32) -> Option<super::ProcSample> {
        None
    }

    pub fn limit_ram(_pid: u32, _megabytes: u32) -> Result<(), String> {
        Err(ONLY_WINDOWS.into())
    }

    pub fn limit_cpu(_pid: u32, _percent: u32) -> Result<(), String> {
        Err(ONLY_WINDOWS.into())
    }
}

/// CPU time and memory for one Roblox process.
#[derive(Debug, Clone, Copy)]
pub struct ProcSample {
    /// User plus kernel CPU time, in 100 ns units.
    pub cpu_100ns: u64,
    /// Physical memory in use, in bytes.
    pub working_set: u64,
}

#[allow(unused_imports)]
pub use imp::{
    cores, limit_cpu, limit_ram, resize_windows, roblox_process_ids, sample, set_priority,
    spread_across_cores,
};
