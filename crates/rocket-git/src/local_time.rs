//! UTC instants on this computer's clock, by the time zone rules of that date.

/// A moment as this computer's clock shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// `unix_ms` on this computer's clock, or `None` when the system can't convert it.
#[cfg(windows)]
pub fn local_time(unix_ms: u64) -> Option<LocalTime> {
    use std::{ffi::c_void, ptr};

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    unsafe extern "system" {
        fn FileTimeToSystemTime(file: *const FileTime, system: *mut SystemTime) -> i32;
        fn SystemTimeToTzSpecificLocalTime(
            zone: *const c_void,
            universal: *const SystemTime,
            local: *mut SystemTime,
        ) -> i32;
    }

    // A FILETIME counts 100 ns ticks from 1601.
    let ticks = unix_ms
        .checked_add(11_644_473_600_000)?
        .checked_mul(10_000)?;
    let file = FileTime {
        low: ticks as u32,
        high: (ticks >> 32) as u32,
    };
    let (mut universal, mut local) = (SystemTime::default(), SystemTime::default());
    // SAFETY: each call gets valid pointers to live, correctly laid out structures, and
    // a null zone selects the computer's current time zone.
    let converted = unsafe {
        FileTimeToSystemTime(&file, &mut universal) != 0
            && SystemTimeToTzSpecificLocalTime(ptr::null(), &universal, &mut local) != 0
    };
    converted.then_some(LocalTime {
        year: local.year,
        month: local.month as u8,
        day: local.day as u8,
        hour: local.hour as u8,
        minute: local.minute as u8,
        second: local.second as u8,
    })
}

#[cfg(not(windows))]
pub fn local_time(_unix_ms: u64) -> Option<LocalTime> {
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn local_time_keeps_minutes_and_seconds_near_utc() {
        // 2026-10-19 06:13:20 UTC. Zones differ by whole hours or quarter hours.
        let time = local_time(1_792_390_400_000).expect("the system converts a recent date");
        assert_eq!(time.year, 2026);
        assert_eq!(time.month, 10);
        assert!((18..=20).contains(&time.day));
        assert_eq!(time.second, 20);
        assert_eq!(time.minute % 15, 13);
    }

    #[test]
    fn a_time_past_the_system_range_is_refused() {
        assert_eq!(local_time(u64::MAX), None);
    }
}
