//! The errors of the GPU, for the whole program (it has one device).
//!
//! - wgpu ends the program at an error that no error scope takes. Here the log names the
//!   error (with the label of the object) and the program goes on: the draw or the pass that
//!   has the error does nothing.
//! - A lost device (a reset of the driver) ends all drawing. The log says so, and with a
//!   window a message box tells the player.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

static ERRORS: AtomicU64 = AtomicU64::new(0);
static LOST: AtomicBool = AtomicBool::new(false);
/// `test_loss` destroys the device: its loss is then handled as a loss for an unknown reason.
static TEST_LOSS: AtomicBool = AtomicBool::new(false);

/// The errors that the log shows in full. After them, one line for each 1000.
const ERRORS_SHOWN: u64 = 20;

/// Sets the handlers of the errors and of the loss of `device`. `window`: the program draws
/// into a window, and a lost device gets a message box.
pub fn watch(device: &wgpu::Device, adapter: &wgpu::AdapterInfo, window: bool) {
    let name = format!("{} ({:?}, driver {} {})", adapter.name, adapter.backend, adapter.driver, adapter.driver_info);
    device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
        let count = ERRORS.fetch_add(1, Ordering::Relaxed) + 1;
        let kind = match &error {
            wgpu::Error::OutOfMemory { .. } => "out of memory",
            wgpu::Error::Validation { .. } => "validation",
            wgpu::Error::Internal { .. } => "internal",
        };
        if count <= ERRORS_SHOWN {
            println!("gpu error {count} ({kind}, adapter {name}): {error}");
            if count == ERRORS_SHOWN {
                println!("gpu error: the log shows no more errors in full, only their count");
            }
        } else if count % 1000 == 0 {
            println!("gpu error: {count} errors");
        }
    }));
    device.set_device_lost_callback(move |reason, message| {
        let destroyed = matches!(reason, wgpu::DeviceLostReason::Destroyed) && !TEST_LOSS.load(Ordering::Relaxed);
        if destroyed {
            // The program destroyed the device itself, at its end.
            return;
        }
        LOST.store(true, Ordering::Relaxed);
        println!("gpu: the device is lost ({reason:?}): {message}");
        if window {
            std::thread::spawn(lost_message);
        }
    });
}

/// The device is lost: nothing draws any more.
pub fn lost() -> bool {
    LOST.load(Ordering::Relaxed)
}

/// The number of errors since the start.
pub fn errors() -> u64 {
    ERRORS.load(Ordering::Relaxed)
}

/// Destroys the device, and its loss is handled as a real one: the test of the loss. The poll
/// gives the loss to the callback, as a submit or a poll does with a real loss.
pub fn test_loss(device: &wgpu::Device) {
    TEST_LOSS.store(true, Ordering::Relaxed);
    device.destroy();
    let _ = device.poll(wgpu::PollType::Poll);
}

#[cfg(windows)]
fn lost_message() {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(window: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    const MB_ICONERROR: u32 = 0x10;
    const MB_SETFOREGROUND: u32 = 0x1_0000;
    const MB_TOPMOST: u32 = 0x4_0000;
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let text = wide(
        "The graphics device was lost: the driver was reset, or it stopped.\n\n\
         The wgpu renderer of MGE XE cannot draw any more. Quit the game and start it again.\n\n\
         The log of the renderer (nirender-host.log in the folder of the game) has the details.",
    );
    let caption = wide("MGE XE");
    unsafe { MessageBoxW(0, text.as_ptr(), caption.as_ptr(), MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST) };
}

#[cfg(not(windows))]
fn lost_message() {}
