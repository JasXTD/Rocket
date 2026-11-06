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

/// `unix_ms` on this computer's clock, through POSIX `localtime_r` and the system's
/// time zone rules (`TZ`, else `/etc/localtime`), or `None` when it can't convert it.
#[cfg(unix)]
pub fn local_time(unix_ms: u64) -> Option<LocalTime> {
    use std::ffi::{c_char, c_int, c_long};

    // POSIX's fields, then the tm_gmtoff and tm_zone that glibc, musl and macOS add.
    #[repr(C)]
    struct Tm {
        second: c_int,
        minute: c_int,
        hour: c_int,
        day: c_int,
        month: c_int,
        year: c_int,
        weekday: c_int,
        year_day: c_int,
        daylight_saving: c_int,
        offset: c_long,
        zone: *const c_char,
    }
    unsafe extern "C" {
        // time_t is a C long on Linux (glibc, and musl on 64-bit targets) and macOS.
        fn localtime_r(time: *const c_long, result: *mut Tm) -> *mut Tm;
    }

    let seconds = c_long::try_from(unix_ms / 1000).ok()?;
    // SAFETY: all zeroes is a valid Tm (integers and a null pointer).
    let mut local: Tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers refer to live values of the right types for the call; the
    // result is only written into `local`, and its zone pointer is never read.
    if unsafe { localtime_r(&seconds, &mut local) }.is_null() {
        return None;
    }
    Some(LocalTime {
        year: u16::try_from(local.year.checked_add(1900)?).ok()?,
        month: u8::try_from(local.month.checked_add(1)?).ok()?,
        day: u8::try_from(local.day).ok()?,
        hour: u8::try_from(local.hour).ok()?,
        minute: u8::try_from(local.minute).ok()?,
        second: u8::try_from(local.second).ok()?,
    })
}

#[cfg(not(any(windows, unix)))]
pub fn local_time(_unix_ms: u64) -> Option<LocalTime> {
    None
}

#[cfg(all(test, any(windows, unix)))]
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

    /// The same instants by GNU date, which reads the same time zone rules: one in
    /// January and one in July, so a daylight-saving zone checks both offsets.
    #[cfg(target_os = "linux")]
    #[test]
    fn local_time_matches_the_date_command() {
        for unix_ms in [1_768_478_400_000_u64, 1_784_116_800_000, 1_792_390_400_000] {
            let output = std::process::Command::new("date")
                .arg("-d")
                .arg(format!("@{}", unix_ms / 1000))
                .arg("+%Y %m %d %H %M %S")
                .output()
                .expect("run date");
            assert!(output.status.success(), "{output:?}");
            let expected: Vec<u32> = String::from_utf8(output.stdout)
                .unwrap()
                .split_whitespace()
                .map(|field| field.parse().unwrap())
                .collect();
            let time = local_time(unix_ms).expect("the system converts a recent date");
            let actual = [time.year.into(), time.month.into(), time.day.into(), time.hour.into(), time.minute.into(), time.second.into()];
            assert_eq!(expected, actual, "{unix_ms}");
        }
    }
}
