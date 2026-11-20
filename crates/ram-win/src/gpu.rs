//! Per-process GPU usage from the documented PDH performance counters.

use std::collections::HashMap;

use windows::core::PCWSTR;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW, PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_FMT_DOUBLE, PDH_FMT_LARGE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
};

use crate::util::wide;

const UTILISATION_PATH: &str = r"\GPU Engine(*)\Utilization Percentage";
const DEDICATED_PATH: &str = r"\GPU Process Memory(*)\Dedicated Usage";

/// Samples GPU engine utilisation and dedicated GPU memory per process.
///
/// Values come from the `GPU Engine` and `GPU Process Memory` counter sets
/// (Windows 10 1709 and later, WDDM 2.x drivers). Utilisation is a rate, so
/// it is available only after the second [`collect`](Self::collect).
pub struct GpuSampler {
    query: PDH_HQUERY,
    utilisation: Option<PDH_HCOUNTER>,
    dedicated: Option<PDH_HCOUNTER>,
    collections: u32,
    usage: HashMap<u32, f32>,
    memory: HashMap<u32, u64>,
    scratch: Vec<u64>,
}

// PDH query handles are not bound to the thread that created them.
unsafe impl Send for GpuSampler {}

impl GpuSampler {
    /// Opens the PDH query. Returns `None` when neither counter exists.
    pub fn new() -> Option<Self> {
        let mut query = PDH_HQUERY::default();
        if unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) } != 0 {
            return None;
        }
        let utilisation = add_counter(query, UTILISATION_PATH);
        let dedicated = add_counter(query, DEDICATED_PATH);
        if utilisation.is_none() && dedicated.is_none() {
            unsafe { PdhCloseQuery(query) };
            return None;
        }
        Some(Self {
            query,
            utilisation,
            dedicated,
            collections: 0,
            usage: HashMap::new(),
            memory: HashMap::new(),
            scratch: Vec::new(),
        })
    }

    /// Performs one PDH collection and refreshes the per-process totals.
    pub fn collect(&mut self) {
        self.usage.clear();
        self.memory.clear();
        if unsafe { PdhCollectQueryData(self.query) } != 0 {
            return;
        }
        self.collections = self.collections.saturating_add(1);

        if let Some(counter) = self.dedicated {
            let memory = &mut self.memory;
            read_array(counter, PDH_FMT_LARGE, &mut self.scratch, |pid, item| {
                let bytes = unsafe { item.FmtValue.Anonymous.largeValue }.max(0) as u64;
                *memory.entry(pid).or_insert(0) += bytes;
            });
        }
        if self.collections >= 2 {
            if let Some(counter) = self.utilisation {
                let usage = &mut self.usage;
                read_array(counter, PDH_FMT_DOUBLE, &mut self.scratch, |pid, item| {
                    let percent = unsafe { item.FmtValue.Anonymous.doubleValue };
                    if percent.is_finite() && percent >= 0.0 {
                        *usage.entry(pid).or_insert(0.0) += percent as f32;
                    }
                });
            }
            for value in self.usage.values_mut() {
                *value = value.clamp(0.0, 100.0);
            }
        }
    }

    /// GPU utilisation (0..=100, summed over the process's engines) and
    /// dedicated GPU memory in bytes from the last collection. Each is `None`
    /// when PDH reported no instance for the process.
    pub fn get(&self, pid: u32) -> (Option<f32>, Option<u64>) {
        (self.usage.get(&pid).copied(), self.memory.get(&pid).copied())
    }
}

impl Drop for GpuSampler {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

fn add_counter(query: PDH_HQUERY, path: &str) -> Option<PDH_HCOUNTER> {
    let path = wide(path);
    let mut counter = PDH_HCOUNTER::default();
    let status = unsafe { PdhAddEnglishCounterW(query, PCWSTR(path.as_ptr()), 0, &mut counter) };
    (status == 0).then_some(counter)
}

/// Reads every instance of a wildcard counter and calls `each` with the
/// owning PID for instances that carry valid data.
fn read_array(
    counter: PDH_HCOUNTER,
    format: PDH_FMT,
    scratch: &mut Vec<u64>,
    mut each: impl FnMut(u32, &PDH_FMT_COUNTERVALUE_ITEM_W),
) {
    // The instance set can grow between the size query and the read.
    for _ in 0..3 {
        let mut size = 0u32;
        let mut count = 0u32;
        let status =
            unsafe { PdhGetFormattedCounterArrayW(counter, format, &mut size, &mut count, None) };
        if status != PDH_MORE_DATA || size == 0 {
            return;
        }
        // u64 storage keeps the item array correctly aligned.
        scratch.clear();
        scratch.resize((size as usize).div_ceil(8), 0);
        let items = scratch.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        let status = unsafe {
            PdhGetFormattedCounterArrayW(counter, format, &mut size, &mut count, Some(items))
        };
        if status == PDH_MORE_DATA {
            continue;
        }
        if status != 0 {
            return;
        }
        let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
        for item in items {
            let valid = item.FmtValue.CStatus == PDH_CSTATUS_VALID_DATA
                || item.FmtValue.CStatus == PDH_CSTATUS_NEW_DATA;
            if !valid || item.szName.is_null() {
                continue;
            }
            let name = String::from_utf16_lossy(unsafe { item.szName.as_wide() });
            if let Some(pid) = pid_from_instance(&name) {
                each(pid, item);
            }
        }
        return;
    }
}

/// Extracts the PID from an instance name such as
/// `pid_1234_luid_0x00000000_0x0000C2D1_phys_0_eng_0_engtype_3D`.
pub(crate) fn pid_from_instance(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("pid_")?;
    let digits = rest.split('_').next()?;
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pid_from_instance_names() {
        assert_eq!(
            pid_from_instance("pid_1234_luid_0x00000000_0x0000C2D1_phys_0_eng_0_engtype_3D"),
            Some(1234)
        );
        assert_eq!(pid_from_instance("pid_77_luid_0x0_0x1_phys_0"), Some(77));
        assert_eq!(pid_from_instance("luid_0x0_0x1_phys_0"), None);
        assert_eq!(pid_from_instance("pid_x_luid"), None);
        assert_eq!(pid_from_instance(""), None);
    }

    #[test]
    fn sampler_never_reports_out_of_range_values() {
        // Passes trivially on machines without GPU counters.
        let Some(mut sampler) = GpuSampler::new() else {
            return;
        };
        sampler.collect();
        assert_eq!(sampler.get(std::process::id()).0, None, "rate needs two collections");
        std::thread::sleep(std::time::Duration::from_millis(300));
        sampler.collect();
        for percent in sampler.usage.values() {
            assert!((0.0..=100.0).contains(percent));
        }
    }
}
