//! The monitors of the computer as the adapters of the plugin's Direct3D 8 (`own_device`), as
//! Direct3D 8 numbers them: one adapter for each monitor, the primary first, each with its own
//! display modes. The engine takes its adapter from the registry value `Adapter`
//! (`NiApplication::ReadSettings` 0x4F54F0), so a game set to a second monitor gets that
//! monitor's modes, and its fullscreen window goes onto that monitor.
//!
//! The engine indexes its list of adapters with that value at once (`NiDX8Renderer::Create`
//! 0x6AA590, "Creation failed: Invalid Adapter"), before `CreateDevice`. A value past the
//! monitors (a second monitor unplugged, the value kept) gets adapters up to it, each one the
//! primary monitor, so that the game starts on the primary ([`adapter_count`]).

#[link(name = "user32")]
extern "system" {
    fn EnumDisplayMonitors(dc: usize, clip: *const i32, callback: unsafe extern "system" fn(usize, usize, *const i32, isize) -> i32, data: isize) -> i32;
    fn GetMonitorInfoA(monitor: usize, info: *mut u8) -> i32;
    fn EnumDisplaySettingsA(device: *const u8, mode: u32, out: *mut u8) -> i32;
}

/// The most adapters that the game's `Adapter` value can ask for: a larger value is not a
/// monitor that was unplugged.
pub const MOST_ADAPTERS: u32 = 8;

/// One monitor: its handle, the name of its display device (`\\.\DISPLAY2`, zero-ended), its
/// place on the desktop (left, top, right, bottom), whether it is the primary, its display
/// modes (width, height, refresh rate) and its current mode.
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub handle: usize,
    pub device: [u8; 32],
    pub rect: [i32; 4],
    pub primary: bool,
    pub modes: Vec<[u32; 3]>,
    pub current: [u32; 3],
}

impl Monitor {
    /// The size of the monitor in the coordinates of this process: its pixels when the process
    /// is DPI-aware, and scaled by the display's scale when it is not (a game without the
    /// compatibility setting "high DPI", on a display at 150 %: 2560 x 1440 for 3840 x 2160).
    /// The window and the back buffer of a fullscreen game take it; the display modes are
    /// the device's pixels.
    pub fn size(&self) -> [u32; 2] {
        [(self.rect[2] - self.rect[0]).max(0) as u32, (self.rect[3] - self.rect[1]).max(0) as u32]
    }

    /// The name of the display device, without the zero.
    pub fn name(&self) -> &str {
        let end = self.device.iter().position(|&byte| byte == 0).unwrap_or(self.device.len());
        core::str::from_utf8(&self.device[..end]).unwrap_or("?")
    }
}

/// The adapters: the primary monitor first, the others in the order of the system.
pub fn adapters(mut monitors: Vec<Monitor>) -> Vec<Monitor> {
    monitors.sort_by_key(|monitor| !monitor.primary);
    monitors
}

/// The display modes of a monitor from its entries (the bits of a pixel, and the mode): the
/// modes of 32 bits, each once, in their order; the current mode when there is none.
pub fn modes(entries: impl IntoIterator<Item = (u32, [u32; 3])>, current: [u32; 3]) -> Vec<[u32; 3]> {
    let mut modes = Vec::new();
    for (bits, mode) in entries {
        if bits == 32 && !modes.contains(&mode) {
            modes.push(mode);
        }
    }
    if modes.is_empty() {
        modes.push(current);
    }
    modes
}

/// The number of adapters that the game sees: one for each monitor (at least one), and up to
/// the game's `Adapter` value when it is past them but below [`MOST_ADAPTERS`].
pub fn adapter_count(monitors: usize, wanted: u32) -> u32 {
    let monitors = monitors.max(1) as u32;
    if wanted >= monitors && wanted < MOST_ADAPTERS {
        wanted + 1
    } else {
        monitors
    }
}

/// The monitor of an adapter: its own, or the primary (0) for an adapter past the monitors.
pub fn monitor_of(adapter: u32, monitors: usize) -> usize {
    if (adapter as usize) < monitors {
        adapter as usize
    } else {
        0
    }
}

/// The size of the back buffer of `CreateDevice` and `Reset`, as Direct3D 8 takes it: the size
/// given, or, for a size of 0, the inside of the window (`client`) when the game is windowed,
/// and the size of the adapter's monitor ([`Monitor::size`]) in fullscreen.
pub fn back_buffer_size(given: [u32; 2], windowed: bool, client: [u32; 2], monitor: [u32; 2]) -> [u32; 2] {
    if given[0] != 0 && given[1] != 0 {
        given
    } else if windowed && client[0] != 0 && client[1] != 0 {
        client
    } else {
        monitor
    }
}

/// The monitors of the computer, with their modes, in the order of [`adapters`]. Empty when
/// the system gives none.
///
/// # Safety
///
/// Calls the Win32 functions of the monitors; any thread.
pub unsafe fn read() -> Vec<Monitor> {
    unsafe extern "system" fn found(handle: usize, _dc: usize, _rect: *const i32, data: isize) -> i32 {
        let handles = &mut *(data as *mut Vec<usize>);
        handles.push(handle);
        1
    }
    let mut handles: Vec<usize> = Vec::new();
    EnumDisplayMonitors(0, core::ptr::null(), found, &mut handles as *mut Vec<usize> as isize);
    let monitors = handles.into_iter().filter_map(|handle| read_monitor(handle)).collect();
    adapters(monitors)
}

/// One monitor by its handle: MONITORINFOEXA (its size at 0, the monitor's rectangle at 4, the
/// flags at 36 with 1 for the primary, the device's name at 40, 72 bytes in all), and the
/// modes of its device.
unsafe fn read_monitor(handle: usize) -> Option<Monitor> {
    let mut info = [0u8; 72];
    info[0..4].copy_from_slice(&72u32.to_le_bytes());
    if GetMonitorInfoA(handle, info.as_mut_ptr()) == 0 {
        return None;
    }
    let int = |at: usize| i32::from_le_bytes([info[at], info[at + 1], info[at + 2], info[at + 3]]);
    let mut device = [0u8; 32];
    device.copy_from_slice(&info[40..72]);
    device[31] = 0;
    // A DEVMODEA has 156 bytes: its size at 36, the bits for each pixel at 104, the width at
    // 108, the height at 112 and the refresh rate at 120.
    let mut mode = [0u8; 256];
    mode[36..38].copy_from_slice(&156u16.to_le_bytes());
    let word = |mode: &[u8], at: usize| u32::from_le_bytes([mode[at], mode[at + 1], mode[at + 2], mode[at + 3]]);
    let width = (int(12) - int(4)).max(0) as u32;
    let height = (int(16) - int(8)).max(0) as u32;
    let mut current = [width, height, 60];
    // ENUM_CURRENT_SETTINGS
    if EnumDisplaySettingsA(device.as_ptr(), 0xFFFF_FFFF, mode.as_mut_ptr()) != 0 {
        current = [word(&mode, 108), word(&mode, 112), word(&mode, 120)];
    }
    let mut entries = Vec::new();
    let mut index = 0;
    while index < 4096 && EnumDisplaySettingsA(device.as_ptr(), index, mode.as_mut_ptr()) != 0 {
        entries.push((word(&mode, 104), [word(&mode, 108), word(&mode, 112), word(&mode, 120)]));
        index += 1;
    }
    Some(Monitor { handle, device, rect: [int(4), int(8), int(12), int(16)], primary: int(36) & 1 != 0, modes: modes(entries, current), current })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, primary: bool, current: [u32; 3]) -> Monitor {
        let mut device = [0u8; 32];
        device[..name.len()].copy_from_slice(name.as_bytes());
        Monitor { handle: 0, device, rect: [0, 0, current[0] as i32, current[1] as i32], primary, modes: vec![current], current }
    }

    #[test]
    fn the_primary_monitor_is_adapter_0() {
        let second = monitor("\\\\.\\DISPLAY2", false, [1920, 1080, 60]);
        let primary = monitor("\\\\.\\DISPLAY1", true, [2560, 1440, 144]);
        let third = monitor("\\\\.\\DISPLAY3", false, [1280, 1024, 75]);
        let list = adapters(vec![second.clone(), primary.clone(), third.clone()]);
        assert_eq!(list, vec![primary, second, third]);
        assert_eq!(list[1].name(), "\\\\.\\DISPLAY2");
    }

    #[test]
    fn a_monitor_has_its_modes_of_32_bits_each_once() {
        let entries = [(32, [1920, 1080, 60]), (16, [800, 600, 60]), (32, [1920, 1080, 60]), (32, [1280, 720, 144])];
        assert_eq!(modes(entries, [1920, 1080, 60]), vec![[1920, 1080, 60], [1280, 720, 144]]);
        // No mode of 32 bits: the current one.
        assert_eq!(modes([(16, [640, 480, 60])], [1024, 768, 60]), vec![[1024, 768, 60]]);
    }

    #[test]
    fn an_adapter_value_past_the_monitors_gets_the_primary() {
        // Two monitors, the game set to the second.
        assert_eq!(adapter_count(2, 1), 2);
        assert_eq!(monitor_of(1, 2), 1);
        // The second monitor unplugged, the value kept: adapter 1 exists and is the primary.
        assert_eq!(adapter_count(1, 1), 2);
        assert_eq!(monitor_of(1, 1), 0);
        // A value that no unplugged monitor gives: the monitors only.
        assert_eq!(adapter_count(1, MOST_ADAPTERS), 1);
        assert_eq!(adapter_count(1, u32::MAX), 1);
        // No monitor read: one adapter.
        assert_eq!(adapter_count(0, 0), 1);
        assert_eq!(monitor_of(0, 0), 0);
    }

    /// The monitors of this computer, as the Win32 calls give them (the offsets of
    /// MONITORINFOEXA and DEVMODEA). Needs a desktop:
    /// `cargo test --release -- --ignored the_monitors_of_this_computer --nocapture`.
    #[test]
    #[ignore = "needs the desktop of a computer"]
    fn the_monitors_of_this_computer() {
        // SAFETY: the Win32 calls of the monitors, on this thread.
        let list = unsafe { read() };
        for monitor in &list {
            println!("{} primary {} rect {:?} current {:?}, {} modes", monitor.name(), monitor.primary, monitor.rect, monitor.current, monitor.modes.len());
        }
        assert!(!list.is_empty() && list[0].primary, "{list:?}");
        for monitor in &list {
            assert!(monitor.name().starts_with(r"\\.\DISPLAY"), "{}", monitor.name());
            // The size can be scaled (a process that is not DPI-aware); the mode is the pixels.
            let size = monitor.size();
            assert!(size[0] > 0 && size[0] <= monitor.current[0] && size[1] > 0 && size[1] <= monitor.current[1], "{size:?} {:?}", monitor.current);
            assert!(monitor.modes.contains(&monitor.current), "{:?}", monitor.modes);
        }
    }

    #[test]
    fn a_size_of_0_is_the_window_when_windowed_and_the_monitor_in_fullscreen() {
        let (client, mode) = ([1600, 900], [2560, 1440]);
        assert_eq!(back_buffer_size([1280, 720], true, client, mode), [1280, 720]);
        assert_eq!(back_buffer_size([1280, 720], false, client, mode), [1280, 720]);
        assert_eq!(back_buffer_size([0, 0], true, client, mode), [1600, 900]);
        assert_eq!(back_buffer_size([0, 0], false, client, mode), [2560, 1440]);
        // A window with no inside yet: the monitor.
        assert_eq!(back_buffer_size([0, 0], true, [0, 0], mode), [2560, 1440]);
    }
}
