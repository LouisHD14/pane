//! System load for the bar meters: CPU, RAM, GPU, network throughput.

use serde::Serialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;
use windows::core::w;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Usage {
    pub cpu: u8,
    pub ram: u8,
    pub gpu: Option<u8>,
    /// Bytes per second.
    pub net_down: u64,
    pub net_up: u64,
}

/// Counters from the previous call.
struct Prev {
    at: Instant,
    cpu: (u64, u64, u64), // idle, kernel (includes idle), user
    net: (u64, u64),      // in, out
    gpu: Option<(PDH_HQUERY, PDH_HCOUNTER)>,
}

// shortcut: state is thread-local, so all calls must come from one thread; move into a struct if two callers appear.
thread_local! {
    static PREV: RefCell<Option<Prev>> = const { RefCell::new(None) };
}

/// Call about once a second from one background thread; rates and CPU % are computed against the previous call.
pub fn sample() -> Usage {
    let now = Instant::now();
    let cpu_now = cpu_times();
    let net_now = net_bytes();
    PREV.with_borrow_mut(|prev| {
        let p = prev.get_or_insert_with(|| Prev { at: now, cpu: cpu_now, net: net_now, gpu: gpu_open() });
        let secs = now.duration_since(p.at).as_secs_f64();
        let usage = Usage {
            cpu: cpu_percent(p.cpu, cpu_now),
            ram: ram(),
            gpu: p.gpu.and_then(|(q, c)| gpu(q, c)),
            net_down: rate(p.net.0, net_now.0, secs),
            net_up: rate(p.net.1, net_now.1, secs),
        };
        (p.at, p.cpu, p.net) = (now, cpu_now, net_now);
        usage
    })
}

/// Busy share of all cores between two (idle, kernel, user) readings; kernel time includes idle.
fn cpu_percent(a: (u64, u64, u64), b: (u64, u64, u64)) -> u8 {
    let idle = b.0.saturating_sub(a.0);
    let total = b.1.saturating_sub(a.1) + b.2.saturating_sub(a.2);
    if total == 0 {
        return 0;
    }
    (100 * total.saturating_sub(idle) / total).min(100) as u8
}

/// Per-second rate; 0 on zero elapsed time or when the counter went backwards (reset, adapter gone).
fn rate(a: u64, b: u64, secs: f64) -> u64 {
    if secs <= 0.0 || b < a {
        return 0;
    }
    ((b - a) as f64 / secs) as u64
}

fn cpu_times() -> (u64, u64, u64) {
    let (mut i, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    match unsafe { GetSystemTimes(Some(&mut i), Some(&mut k), Some(&mut u)) } {
        Ok(()) => (ft(i), ft(k), ft(u)),
        Err(_) => (0, 0, 0),
    }
}

fn ram() -> u8 {
    let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    match unsafe { GlobalMemoryStatusEx(&mut m) } {
        Ok(()) => m.dwMemoryLoad.min(100) as u8,
        Err(_) => 0,
    }
}

/// Summed in/out octets of hardware adapters that are up (skips loopback and virtual adapters).
fn net_bytes() -> (u64, u64) {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    if unsafe { GetIfTable2(&mut table) }.is_err() {
        return (0, 0);
    }
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize) };
    let sum = rows
        .iter()
        .filter(|r| r.OperStatus == IfOperStatusUp && r.InterfaceAndOperStatusFlags._bitfield & 1 != 0)
        .fold((0, 0), |(i, o), r| (i + r.InOctets, o + r.OutOctets));
    unsafe { FreeMibTable(table as _) };
    sum
}

/// Opens the 3D-engine query once; None when PDH or the GPU counters are unavailable.
// shortcut: the query is never closed (lives as long as the thread); add PdhCloseQuery if sampling ever stops.
fn gpu_open() -> Option<(PDH_HQUERY, PDH_HCOUNTER)> {
    let (mut q, mut c) = (PDH_HQUERY::default(), PDH_HCOUNTER::default());
    unsafe {
        if PdhOpenQueryW(None, 0, &mut q) != 0
            || PdhAddEnglishCounterW(q, w!(r"\GPU Engine(*engtype_3D)\Utilization Percentage"), 0, &mut c) != 0
        {
            return None;
        }
        PdhCollectQueryData(q); // rate counter: values arrive from the next collect
    }
    Some((q, c))
}

/// Like Task Manager: 3D utilization summed per adapter (luid), busiest adapter wins.
fn gpu(q: PDH_HQUERY, c: PDH_HCOUNTER) -> Option<u8> {
    let (mut size, mut count) = (0u32, 0u32);
    unsafe {
        if PdhCollectQueryData(q) != 0 {
            return None;
        }
        PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u64; size as usize / 8 + 1]; // 8-byte aligned
        let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
            return None;
        }
        let items = std::slice::from_raw_parts(items, count as usize);
        Some(gpu_max(items.iter().map(|i| (i.szName.to_string().unwrap_or_default(), i.FmtValue.Anonymous.doubleValue))))
    }
}

/// Sums engine values per adapter; instance names look like "pid_4_luid_0x0_0xD1B5_phys_0_eng_0_engtype_3D".
fn gpu_max(items: impl Iterator<Item = (String, f64)>) -> u8 {
    let mut per: HashMap<String, f64> = HashMap::new();
    for (name, v) in items {
        let luid = name.split("_phys").next().and_then(|s| s.split("luid_").nth(1)).unwrap_or("").to_string();
        *per.entry(luid).or_default() += v;
    }
    per.values().fold(0f64, |m, &v| m.max(v)).clamp(0.0, 100.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_math() {
        // 100 ticks total (kernel 60 incl. idle 50, user 40) -> 50 % busy.
        assert_eq!(cpu_percent((0, 0, 0), (50, 60, 40)), 50);
        assert_eq!(cpu_percent((5, 5, 5), (5, 5, 5)), 0);
    }

    #[test]
    fn rate_math() {
        assert_eq!(rate(1000, 3000, 2.0), 1000);
        assert_eq!(rate(1000, 3000, 0.0), 0);
        assert_eq!(rate(3000, 10, 1.0), 0);
    }

    #[test]
    fn gpu_per_adapter() {
        let n = |l: &str, e: u8| format!("pid_{e}_luid_0x0_{l}_phys_0_eng_0_engtype_3D");
        let items = vec![(n("0xA", 1), 30.0), (n("0xA", 2), 25.0), (n("0xB", 3), 40.0)];
        assert_eq!(gpu_max(items.into_iter()), 55);
    }

    #[test]
    #[ignore]
    fn usage_live() {
        for _ in 0..3 {
            let t = Instant::now();
            let u = sample();
            println!("{u:?} in {:?}", t.elapsed());
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}
