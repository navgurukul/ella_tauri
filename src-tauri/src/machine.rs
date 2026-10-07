//! What the computer was doing while Ella worked, for her telemetry.
//!
//! On 2026-10-07 the Windows test laptop ran the same build twice in one
//! morning, and the second time everything took about 2.7 times as long:
//! Canary, which never touches the language model, as much as the model.
//! Nothing in `latency.jsonl` could say why. The laptop might have been
//! unplugged, held back by its power mode, hot, or busy with something else,
//! and the file could not even say whether it was the same laptop. So each
//! launch now says what it runs on. Each turn says how the computer was
//! powered, how fast its CPU was let run, and how busy it was, by Ella and by
//! everything else.
//!
//! Windows is where testers run Ella, and the only place a turn's state is
//! read. Elsewhere a launch still describes the computer as far as it can.

use serde::Serialize;

/// The computer a launch runs on, for its `ella_launch` event.
#[derive(Debug, Clone, Serialize)]
pub struct Description {
    /// `Windows 11 build 22631.4317 (23H2)`, `macOS 15.1`.
    pub os: String,
    pub arch: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cores_physical: Option<usize>,
    pub cores_logical: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
}

/// How the computer stood over one stretch of Ella's work, a turn or an
/// assessment, or at one moment. A part the system would not give is left
/// out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct State {
    /// Plugged in, or on its battery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_ac: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_pct: Option<u8>,
    /// Windows' battery saver, which holds the CPU back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_saver: Option<bool>,
    /// The power mode slider: `best_efficiency`, `balanced`,
    /// `better_performance` or `best_performance`, or the scheme's GUID when
    /// it is none of those.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_mode: Option<String>,
    /// The clock Windows reports for the CPU, averaged over its cores. Many
    /// machines report their rated clock here whatever they run at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_mhz: Option<u32>,
    /// The rated top clock.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_mhz_max: Option<u32>,
    /// The lowest limit any core is held to. This is what drops when a
    /// laptop is throttled for heat or power.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_mhz_limit: Option<u32>,
    /// How busy the whole CPU was over the stretch, all cores together.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_busy_pct: Option<u8>,
    /// How much of the CPU went to Ella: this process, with Canary in it,
    /// and llama-server and Piper. The rest of `cpu_busy_pct` was
    /// something else.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_ella_pct: Option<u8>,
    /// Memory free at the end of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_free_mb: Option<u64>,
}

/// CPU time used up to one moment, by the whole computer and by Ella, to
/// compare with a later moment in `state_since`.
pub struct Sample(imp::Sample);

/// Takes a `Sample`: a few system calls, cheap enough to take at the start of
/// every turn.
pub fn sample() -> Sample {
    Sample(imp::sample())
}

/// How the computer stood from `since` until now. `None` where nothing could
/// be read.
pub fn state_since(since: &Sample) -> Option<State> {
    Some(imp::state_since(&since.0)).filter(|state| *state != State::default())
}

/// How the computer stands now: its power and its clock, but not how busy it
/// has been, which needs a stretch of time to measure.
pub fn state_now() -> Option<State> {
    Some(imp::state_now()).filter(|state| *state != State::default())
}

/// Counts process `pid` as Ella's own when telling her CPU time from
/// everything else's: llama-server, the resident Piper.
pub fn watch(pid: u32) {
    imp::watch(pid);
}

pub fn describe() -> Description {
    imp::describe()
}

/// `part` as a whole percentage of `whole`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn percent(part: u64, whole: u64) -> Option<u8> {
    (whole > 0).then(|| {
        (part as f64 * 100.0 / whole as f64)
            .round()
            .clamp(0.0, 100.0) as u8
    })
}

/// The power mode a scheme GUID stands for, written the usual way. These are
/// the overlays Windows' power slider moves between. Any other GUID is given
/// as it is.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn power_mode_name(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> String {
    let guid = format!(
        "{data1:08x}-{data2:04x}-{data3:04x}-{:02x}{:02x}-{}",
        data4[0],
        data4[1],
        data4[2..]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    match guid.as_str() {
        "961cc777-2547-4f9d-8174-7d86181b8a7a" => "best_efficiency".into(),
        "00000000-0000-0000-0000-000000000000" => "balanced".into(),
        "3af9b8d9-7c97-431d-ad78-34a8bfea439f" => "better_performance".into(),
        "ded574b5-45a0-4f42-8737-46345c09c238" => "best_performance".into(),
        _ => guid,
    }
}

/// `Windows 11 build 22631.4317 (23H2)`. The registry's product name says
/// Windows 10 on Windows 11 too, so the build number decides.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_name(build: &str, revision: Option<u32>, display: Option<&str>) -> String {
    let release = match build.parse::<u32>() {
        Ok(number) if number >= 22_000 => "Windows 11",
        Ok(_) => "Windows 10",
        Err(_) => "Windows",
    };
    let revision = revision
        .map(|revision| format!(".{revision}"))
        .unwrap_or_default();
    let display = display
        .map(|display| format!(" ({display})"))
        .unwrap_or_default();
    format!("{release} build {build}{revision}{display}")
}

fn logical_cores() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
}

#[cfg(target_os = "windows")]
mod imp {
    use std::{
        mem, ptr,
        sync::{Mutex, OnceLock, PoisonError},
    };

    use windows_sys::{
        core::GUID,
        Win32::{
            Foundation::{CloseHandle, ERROR_SUCCESS, FILETIME},
            System::{
                LibraryLoader::{GetProcAddress, LoadLibraryW},
                Power::{
                    CallNtPowerInformation, GetSystemPowerStatus, ProcessorInformation,
                    PROCESSOR_POWER_INFORMATION, SYSTEM_POWER_STATUS,
                },
                Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ},
                SystemInformation::{
                    GetSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
                },
                Threading::{
                    GetCurrentProcess, GetProcessTimes, GetSystemTimes, OpenProcess,
                    PROCESS_QUERY_LIMITED_INFORMATION,
                },
            },
        },
    };

    use super::{logical_cores, percent, power_mode_name, windows_name, Description, State};

    /// Ella's other processes, by id. A Piper started again leaves its old id
    /// behind, so only the latest few are kept.
    static WATCHED: Mutex<Vec<u32>> = Mutex::new(Vec::new());
    const WATCHED_KEPT: usize = 8;

    const WINDOWS_NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    const FIRST_CPU: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";

    pub struct Sample {
        /// Busy and total CPU time, all cores together, in 100 ns ticks.
        system: Option<(u64, u64)>,
        /// CPU time of each of Ella's processes, by id: 0 for this one.
        ella: Vec<(u32, u64)>,
    }

    pub fn watch(pid: u32) {
        let mut watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
        if !watched.contains(&pid) {
            watched.push(pid);
        }
        let excess = watched.len().saturating_sub(WATCHED_KEPT);
        watched.drain(..excess);
    }

    pub fn sample() -> Sample {
        let watched = WATCHED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let ella = std::iter::once((0, None))
            .chain(watched.into_iter().map(|pid| (pid, Some(pid))))
            .filter_map(|(key, pid)| Some((key, process_ticks(pid)?)))
            .collect();
        Sample {
            system: system_ticks(),
            ella,
        }
    }

    pub fn state_since(since: &Sample) -> State {
        let now = sample();
        let mut state = state_now();
        if let (Some((busy_then, total_then)), Some((busy_now, total_now))) =
            (since.system, now.system)
        {
            let total = total_now.saturating_sub(total_then);
            // A process is counted only over the stretch it was seen at both
            // ends of.
            let ella: u64 = now
                .ella
                .iter()
                .filter_map(|(key, ticks)| {
                    let (_, before) = since.ella.iter().find(|(earlier, _)| earlier == key)?;
                    Some(ticks.saturating_sub(*before))
                })
                .sum();
            state.cpu_busy_pct = percent(busy_now.saturating_sub(busy_then), total);
            state.cpu_ella_pct = percent(ella, total);
        }
        state
    }

    pub fn state_now() -> State {
        let mut state = State::default();
        if let Some(power) = power_status() {
            state.on_ac = match power.ACLineStatus {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            };
            // 128 means there is no battery, 255 that Windows cannot tell.
            state.battery_pct = (power.BatteryFlag & 128 == 0 && power.BatteryLifePercent <= 100)
                .then_some(power.BatteryLifePercent);
            state.battery_saver = Some(power.SystemStatusFlag == 1);
        }
        state.power_mode = power_mode();
        if let Some((current, max, limit)) = clock() {
            state.cpu_mhz = Some(current);
            state.cpu_mhz_max = Some(max);
            state.cpu_mhz_limit = Some(limit);
        }
        state.ram_free_mb = memory().map(|(_, free)| free);
        state
    }

    pub fn describe() -> Description {
        let os = match registry_string(WINDOWS_NT, "CurrentBuild") {
            Some(build) => windows_name(
                &build,
                registry_dword(WINDOWS_NT, "UBR"),
                registry_string(WINDOWS_NT, "DisplayVersion").as_deref(),
            ),
            None => "Windows".into(),
        };
        Description {
            os,
            arch: std::env::consts::ARCH,
            cpu: registry_string(FIRST_CPU, "ProcessorNameString"),
            cores_physical: Some(num_cpus::get_physical()).filter(|cores| *cores > 0),
            cores_logical: logical_cores(),
            ram_mb: memory().map(|(total, _)| total),
        }
    }

    const NO_TIME: FILETIME = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };

    fn ticks(time: &FILETIME) -> u64 {
        (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
    }

    fn system_ticks() -> Option<(u64, u64)> {
        let (mut idle, mut kernel, mut user) = (NO_TIME, NO_TIME, NO_TIME);
        // SAFETY: three writable FILETIMEs, which is all the call takes.
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return None;
        }
        // Kernel time counts the idle time too.
        let total = ticks(&kernel) + ticks(&user);
        Some((total.saturating_sub(ticks(&idle)), total))
    }

    /// CPU time of process `pid`, or of this one, in 100 ns ticks.
    fn process_ticks(pid: Option<u32>) -> Option<u64> {
        // SAFETY: this process's pseudo handle needs no closing; any other is
        // opened here, only to be asked its times, and closed before
        // returning.
        let handle = unsafe {
            match pid {
                None => GetCurrentProcess(),
                Some(pid) => OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid),
            }
        };
        if handle.is_null() {
            return None;
        }
        let (mut creation, mut exit, mut kernel, mut user) = (NO_TIME, NO_TIME, NO_TIME, NO_TIME);
        // SAFETY: a handle opened for querying, and four writable FILETIMEs.
        let read =
            unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
        if pid.is_some() {
            // SAFETY: opened above and not used after this.
            unsafe { CloseHandle(handle) };
        }
        (read != 0).then(|| ticks(&kernel) + ticks(&user))
    }

    fn power_status() -> Option<SYSTEM_POWER_STATUS> {
        // SAFETY: plain data, for which all zeroes is a valid value.
        let mut status: SYSTEM_POWER_STATUS = unsafe { mem::zeroed() };
        // SAFETY: one writable SYSTEM_POWER_STATUS.
        (unsafe { GetSystemPowerStatus(&mut status) } != 0).then_some(status)
    }

    /// The power mode slider's overlay. `PowerGetEffectiveOverlayScheme`
    /// came with Windows 10 1709 and is looked up when first wanted, so an
    /// older Windows still starts Ella and only goes without this.
    fn power_mode() -> Option<String> {
        type Effective = unsafe extern "system" fn(*mut GUID) -> u32;
        static EFFECTIVE: OnceLock<Option<Effective>> = OnceLock::new();
        let effective = (*EFFECTIVE.get_or_init(|| {
            let library: Vec<u16> = "powrprof.dll".encode_utf16().chain([0]).collect();
            // SAFETY: a NUL-terminated name. The library is never freed, so
            // the function taken from it stays valid for the life of the
            // process, and it has the signature Windows documents for it.
            unsafe {
                let module = LoadLibraryW(library.as_ptr());
                if module.is_null() {
                    return None;
                }
                GetProcAddress(module, c"PowerGetEffectiveOverlayScheme".as_ptr().cast()).map(
                    |found| {
                        mem::transmute::<unsafe extern "system" fn() -> isize, Effective>(found)
                    },
                )
            }
        }))?;
        let mut guid = GUID {
            data1: 0,
            data2: 0,
            data3: 0,
            data4: [0; 8],
        };
        // SAFETY: one writable GUID, which is all the call takes.
        if unsafe { effective(&mut guid) } != ERROR_SUCCESS {
            return None;
        }
        Some(power_mode_name(
            guid.data1, guid.data2, guid.data3, guid.data4,
        ))
    }

    /// The average clock Windows reports across the cores, the rated top, and
    /// the lowest limit any core is held to, in MHz.
    fn clock() -> Option<(u32, u32, u32)> {
        // SAFETY: plain data, filled in by the call.
        let processors = unsafe {
            let mut info: SYSTEM_INFO = mem::zeroed();
            GetSystemInfo(&mut info);
            info.dwNumberOfProcessors as usize
        };
        if processors == 0 {
            return None;
        }
        let mut cores = vec![
            PROCESSOR_POWER_INFORMATION {
                Number: 0,
                MaxMhz: 0,
                CurrentMhz: 0,
                MhzLimit: 0,
                MaxIdleState: 0,
                CurrentIdleState: 0,
            };
            processors
        ];
        let size = (processors * mem::size_of::<PROCESSOR_POWER_INFORMATION>()) as u32;
        // SAFETY: the buffer holds one entry per processor, as the call asks,
        // and is `size` bytes long.
        let status = unsafe {
            CallNtPowerInformation(
                ProcessorInformation,
                ptr::null(),
                0,
                cores.as_mut_ptr().cast(),
                size,
            )
        };
        if status != 0 {
            return None;
        }
        let current = cores
            .iter()
            .map(|core| u64::from(core.CurrentMhz))
            .sum::<u64>()
            / processors as u64;
        let max = cores.iter().map(|core| core.MaxMhz).max()?;
        let limit = cores.iter().map(|core| core.MhzLimit).min()?;
        Some((current as u32, max, limit))
    }

    /// Physical memory in all and free, in MB.
    fn memory() -> Option<(u64, u64)> {
        // SAFETY: plain data, for which all zeroes is a valid value.
        let mut status: MEMORYSTATUSEX = unsafe { mem::zeroed() };
        status.dwLength = mem::size_of::<MEMORYSTATUSEX>() as u32;
        // SAFETY: one writable MEMORYSTATUSEX with its length set, as the
        // call requires.
        (unsafe { GlobalMemoryStatusEx(&mut status) } != 0)
            .then_some((status.ullTotalPhys >> 20, status.ullAvailPhys >> 20))
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    fn registry_string(key: &str, value: &str) -> Option<String> {
        let (key, value) = (wide(key), wide(value));
        let mut buffer = vec![0_u16; 256];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: NUL-terminated names, and a buffer `size` bytes long.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let length = (size as usize / 2).min(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..length]);
        let text = text.trim_end_matches('\0').trim();
        (!text.is_empty()).then(|| text.to_owned())
    }

    fn registry_dword(key: &str, value: &str) -> Option<u32> {
        let (key, value) = (wide(key), wide(value));
        let mut data = 0_u32;
        let mut size = mem::size_of::<u32>() as u32;
        // SAFETY: NUL-terminated names, and one writable u32 of `size` bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                ptr::null_mut(),
                (&mut data as *mut u32).cast(),
                &mut size,
            )
        };
        (status == ERROR_SUCCESS).then_some(data)
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::{logical_cores, Description, State};

    pub struct Sample;

    pub fn sample() -> Sample {
        Sample
    }

    pub fn state_since(_since: &Sample) -> State {
        State::default()
    }

    pub fn state_now() -> State {
        State::default()
    }

    pub fn watch(_pid: u32) {}

    #[cfg(target_os = "macos")]
    pub fn describe() -> Description {
        Description {
            os: sysctl_string("kern.osproductversion")
                .map(|version| format!("macOS {version}"))
                .unwrap_or_else(|| "macOS".into()),
            arch: std::env::consts::ARCH,
            cpu: sysctl_string("machdep.cpu.brand_string"),
            cores_physical: sysctl_number::<libc::c_int>("hw.physicalcpu")
                .map(|cores| cores as usize),
            cores_logical: logical_cores(),
            ram_mb: sysctl_number::<u64>("hw.memsize").map(|bytes| bytes >> 20),
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn describe() -> Description {
        Description {
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH,
            cpu: None,
            cores_physical: Some(num_cpus::get_physical()).filter(|cores| *cores > 0),
            cores_logical: logical_cores(),
            ram_mb: None,
        }
    }

    #[cfg(target_os = "macos")]
    fn sysctl_string(name: &str) -> Option<String> {
        let name = std::ffi::CString::new(name).ok()?;
        let mut size = 0;
        // SAFETY: a NUL-terminated name and no buffer, which asks only for
        // the size the value needs.
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 || size == 0 {
            return None;
        }
        let mut buffer = vec![0_u8; size];
        // SAFETY: a buffer of the `size` bytes asked for above.
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 {
            return None;
        }
        let text = String::from_utf8_lossy(&buffer[..size.min(buffer.len())]);
        let text = text.trim_end_matches('\0').trim();
        (!text.is_empty()).then(|| text.to_owned())
    }

    /// A number-valued sysctl, `T` being the type the name holds.
    #[cfg(target_os = "macos")]
    fn sysctl_number<T: Copy + Default>(name: &str) -> Option<T> {
        let name = std::ffi::CString::new(name).ok()?;
        let mut value = T::default();
        let mut size = std::mem::size_of::<T>();
        // SAFETY: `value` and `size` describe one writable `T`, which is what
        // each name asked for holds.
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&mut value as *mut T).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        (status == 0 && size == std::mem::size_of::<T>()).then_some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_power_slider_overlays_have_names_and_any_other_scheme_its_guid() {
        assert_eq!(
            power_mode_name(
                0x961c_c777,
                0x2547,
                0x4f9d,
                [0x81, 0x74, 0x7d, 0x86, 0x18, 0x1b, 0x8a, 0x7a]
            ),
            "best_efficiency"
        );
        assert_eq!(power_mode_name(0, 0, 0, [0; 8]), "balanced");
        assert_eq!(
            power_mode_name(
                0xded5_74b5,
                0x45a0,
                0x4f42,
                [0x87, 0x37, 0x46, 0x34, 0x5c, 0x09, 0xc2, 0x38]
            ),
            "best_performance"
        );
        assert_eq!(
            power_mode_name(0x0123_4567, 0x89ab, 0xcdef, [1, 2, 3, 4, 5, 6, 7, 8]),
            "01234567-89ab-cdef-0102-030405060708"
        );
    }

    #[test]
    fn windows_is_named_by_its_build_not_its_product_name() {
        assert_eq!(
            windows_name("22631", Some(4317), Some("23H2")),
            "Windows 11 build 22631.4317 (23H2)"
        );
        assert_eq!(
            windows_name("19045", Some(5011), Some("22H2")),
            "Windows 10 build 19045.5011 (22H2)"
        );
        assert_eq!(windows_name("19045", None, None), "Windows 10 build 19045");
    }

    #[test]
    fn a_share_of_no_time_is_unknown_and_never_past_whole() {
        assert_eq!(percent(1, 0), None);
        assert_eq!(percent(1, 4), Some(25));
        assert_eq!(percent(5, 4), Some(100));
    }

    #[test]
    fn this_computer_describes_itself() {
        let description = describe();
        assert!(!description.os.is_empty());
        assert!(description.cores_logical >= 1);
        #[cfg(target_os = "macos")]
        {
            assert!(description.os.starts_with("macOS "), "{}", description.os);
            assert!(description.cpu.is_some());
            assert!(description.ram_mb.unwrap_or_default() > 1_024);
        }
    }
}
