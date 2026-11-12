//! System load for the bar meters: CPU, RAM, GPU, network throughput.

use serde::Serialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhGetFormattedCounterValue, PdhOpenQueryW,
    PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_FMT, PDH_HCOUNTER, PDH_HQUERY,
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
    pdh: Option<Pdh>,
}

/// One PDH query for the counters Task Manager shows. Either counter may be missing.
#[derive(Clone, Copy)]
struct Pdh {
    query: PDH_HQUERY,
    /// `\Processor Information(_Total)\% Processor Utility`: Task Manager's CPU % (counts turbo, so it can pass 100).
    cpu: Option<PDH_HCOUNTER>,
    /// `\GPU Engine(*)\Utilization Percentage`: every engine of every adapter.
    gpu: Option<PDH_HCOUNTER>,
}

/// Lets "% Processor Utility" go past 100 (turbo); not in the windows crate.
const PDH_FMT_NOCAP100: PDH_FMT = PDH_FMT(0x0000_8000);

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
        let p = prev.get_or_insert_with(|| Prev { at: now, cpu: cpu_now, net: net_now, pdh: pdh_open() });
        let secs = now.duration_since(p.at).as_secs_f64();
        let pdh = p.pdh.filter(|d| unsafe { PdhCollectQueryData(d.query) } == 0);
        let usage = Usage {
            // Task Manager's number when the counter exists; plain busy time otherwise.
            cpu: pdh.and_then(|d| d.cpu).and_then(cpu_utility).unwrap_or_else(|| cpu_percent(p.cpu, cpu_now)),
            ram: ram(),
            gpu: pdh.and_then(|d| d.gpu).and_then(gpu),
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

/// Opens the counter query once; None when PDH isn't available. Values arrive from the second collect on.
// shortcut: the query is never closed (lives as long as the thread); add PdhCloseQuery if sampling ever stops.
fn pdh_open() -> Option<Pdh> {
    let mut query = PDH_HQUERY::default();
    unsafe {
        if PdhOpenQueryW(None, 0, &mut query) != 0 {
            return None;
        }
        let add = |path: PCWSTR| {
            let mut c = PDH_HCOUNTER::default();
            (PdhAddEnglishCounterW(query, path, 0, &mut c) == 0).then_some(c)
        };
        let cpu = add(w!(r"\Processor Information(_Total)\% Processor Utility"));
        let gpu = add(w!(r"\GPU Engine(*)\Utilization Percentage"));
        PdhCollectQueryData(query); // rate counters: values arrive from the next collect
        Some(Pdh { query, cpu, gpu })
    }
}

fn cpu_utility(c: PDH_HCOUNTER) -> Option<u8> {
    let mut v = PDH_FMT_COUNTERVALUE::default();
    unsafe {
        if PdhGetFormattedCounterValue(c, PDH_FMT(PDH_FMT_DOUBLE.0 | PDH_FMT_NOCAP100.0), None, &mut v) != 0 || v.CStatus != 0 {
            return None;
        }
        Some(v.Anonymous.doubleValue.clamp(0.0, 100.0).round() as u8)
    }
}

/// GPU % like Task Manager: per adapter, the busiest engine type (3D, Copy, Video Decode, Compute, ...), each type
/// summed over its engines and processes; the busiest adapter wins. Counting only 3D (before) showed ~0 % on
/// Intel iGPUs while a video played, because that runs on the Video Decode / Video Processing engines.
fn gpu(c: PDH_HCOUNTER) -> Option<u8> {
    let (mut size, mut count) = (0u32, 0u32);
    unsafe {
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
        Some(gpu_max(
            items
                .iter()
                .filter(|i| i.FmtValue.CStatus == 0)
                .map(|i| (i.szName.to_string().unwrap_or_default(), i.FmtValue.Anonymous.doubleValue)),
        ))
    }
}

/// Sums per (adapter, engine type), returns the busiest; names look like
/// "pid_4_luid_0x0_0xD1B5_phys_0_eng_0_engtype_3D".
fn gpu_max(items: impl Iterator<Item = (String, f64)>) -> u8 {
    let mut per: HashMap<(String, String), f64> = HashMap::new();
    for (name, v) in items {
        let luid = name.split("_phys").next().and_then(|s| s.split("luid_").nth(1)).unwrap_or("").to_string();
        let engine = name.rsplit("engtype_").next().unwrap_or("").to_string();
        *per.entry((luid, engine)).or_default() += v;
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
        // A video on an iGPU: 3D idle, Video Decode busy -> the busiest engine type counts, not their sum.
        let video = vec![
            ("pid_1_luid_0x0_0xA_phys_0_eng_0_engtype_3D".to_string(), 2.0),
            ("pid_1_luid_0x0_0xA_phys_0_eng_5_engtype_VideoDecode".to_string(), 35.0),
            ("pid_2_luid_0x0_0xA_phys_0_eng_7_engtype_VideoProcessing".to_string(), 20.0),
        ];
        assert_eq!(gpu_max(video.into_iter()), 35);
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
