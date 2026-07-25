use std::{
    mem,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::FILETIME,
    System::{
        ProcessStatus::{
            K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
        },
        Threading::{GetCurrentProcess, GetProcessHandleCount, GetProcessTimes},
    },
};

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessUsage {
    pub cpu_percent: f64,
    pub private_bytes: u64,
    pub working_set_bytes: u64,
    pub handle_count: u32,
    pub uptime: Duration,
}

pub struct UsageSampler {
    started_at: Instant,
    last_sample_at: Instant,
    last_cpu_time: u64,
    processor_count: f64,
}

impl UsageSampler {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            started_at: now,
            last_sample_at: now,
            last_cpu_time: process_cpu_time(),
            processor_count: std::thread::available_parallelism()
                .map(|value| value.get() as f64)
                .unwrap_or(1.0),
        }
    }

    pub fn sample(&mut self) -> ProcessUsage {
        let now = Instant::now();
        let cpu_time = process_cpu_time();
        let elapsed = now.duration_since(self.last_sample_at).as_secs_f64();
        let cpu_delta = cpu_time.saturating_sub(self.last_cpu_time) as f64 / 10_000_000.0;
        let cpu_percent = if elapsed > 0.0 {
            (cpu_delta / elapsed / self.processor_count * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        self.last_sample_at = now;
        self.last_cpu_time = cpu_time;

        let (private_bytes, working_set_bytes) = process_memory();
        let mut handle_count = 0;
        unsafe {
            GetProcessHandleCount(GetCurrentProcess(), &mut handle_count);
        }
        ProcessUsage {
            cpu_percent,
            private_bytes,
            working_set_bytes,
            handle_count,
            uptime: now.duration_since(self.started_at),
        }
    }
}

fn process_cpu_time() -> u64 {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return 0;
    }
    filetime_value(kernel).saturating_add(filetime_value(user))
}

fn process_memory() -> (u64, u64) {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )
    };
    if ok == 0 {
        (0, 0)
    } else {
        (counters.PrivateUsage as u64, counters.WorkingSetSize as u64)
    }
}

fn filetime_value(value: FILETIME) -> u64 {
    ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
}
