//! Typing into the focused field, per platform.
//!
//! The credential is injected as synthetic keyboard input, which works
//! regardless of which application owns focus (this is what a global
//! autofill/keystroke tool does; there is no cross-platform "autofill
//! provider" API on desktop).

use crate::Result;

pub trait Typer {
    fn type_text(&mut self, text: &str) -> Result<()>;
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{Result, Typer};
    use std::ffi::c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGEventCreateKeyboardEvent(
            source: *const c_void,
            virtual_key: u16,
            key_down: bool,
        ) -> *mut c_void;
        fn CGEventKeyboardSetUnicodeString(event: *mut c_void, length: usize, string: *const u16);
        fn CGEventPost(tap: i32, event: *mut c_void);
        fn CFRelease(cf: *mut c_void);
        fn AXIsProcessTrusted() -> bool;
    }

    pub struct MacTyper;

    pub fn is_trusted() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    impl Typer for MacTyper {
        fn type_text(&mut self, text: &str) -> Result<()> {
            let chars: Vec<u16> = text.encode_utf16().collect();
            for chunk in chars.chunks(64) {
                unsafe {
                    let down = CGEventCreateKeyboardEvent(std::ptr::null(), 0, true);
                    CGEventKeyboardSetUnicodeString(down, chunk.len(), chunk.as_ptr());
                    CGEventPost(0, down);
                    CFRelease(down);
                    let up = CGEventCreateKeyboardEvent(std::ptr::null(), 0, false);
                    CGEventPost(0, up);
                    CFRelease(up);
                }
            }
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{Result, Typer};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::unix::io::AsRawFd;

    const UI_SET_EVBIT: libc::c_ulong = 0x40045564;
    const UI_SET_KEYBIT: libc::c_ulong = 0x40045565;
    const UI_DEV_SETUP: libc::c_ulong = 0x405C5503;
    const UI_DEV_CREATE: libc::c_ulong = 0x5501;
    const UI_DEV_DESTROY: libc::c_ulong = 0x5502;

    const EV_SYN: u16 = 0x00;
    const EV_KEY: u16 = 0x01;
    const SYN_REPORT: u16 = 0x00;

    const BUS_VIRTUAL: u16 = 0x06;
    const UINPUT_MAX_NAME_SIZE: usize = 80;

    const KEY_LEFTSHIFT: u16 = 42;
    const KEY_RIGHTSHIFT: u16 = 54;

    #[repr(C)]
    struct InputId {
        bustype: u16,
        vendor: u16,
        product: u16,
        version: u16,
    }

    #[repr(C)]
    struct UInputSetup {
        id: InputId,
        name: [u8; UINPUT_MAX_NAME_SIZE],
        ff_effects_max: u32,
    }

    #[repr(C)]
    struct InputEvent {
        time: libc::timeval,
        type_: u16,
        code: u16,
        value: i32,
    }

    pub struct LinuxTyper {
        dev: std::fs::File,
    }

    impl LinuxTyper {
        pub fn new() -> Result<Self> {
            let dev = OpenOptions::new()
                .write(true)
                .read(true)
                .open("/dev/uinput")
                .map_err(|e| format!("open /dev/uinput: {e}"))?;
            unsafe {
                let fd = dev.as_raw_fd();
                libc::ioctl(fd, UI_SET_EVBIT, EV_KEY as libc::c_ulong);
                for code in 1..=120u16 {
                    libc::ioctl(fd, UI_SET_KEYBIT, code as libc::c_ulong);
                }
                let mut name = [0u8; UINPUT_MAX_NAME_SIZE];
                name[.."cr-autofill".len()].copy_from_slice(b"cr-autofill");
                let mut setup = UInputSetup {
                    id: InputId {
                        bustype: BUS_VIRTUAL,
                        vendor: 0,
                        product: 0,
                        version: 0,
                    },
                    name,
                    ff_effects_max: 0,
                };
                libc::ioctl(fd, UI_DEV_SETUP, &mut setup as *mut UInputSetup);
                libc::ioctl(fd, UI_DEV_CREATE, 0);
            }
            Ok(Self { dev })
        }

        fn emit(&mut self, type_: u16, code: u16, value: i32) {
            let ev = InputEvent {
                time: libc::timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_,
                code,
                value,
            };
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    &ev as *const InputEvent as *const u8,
                    std::mem::size_of::<InputEvent>(),
                )
            };
            let _ = self.dev.write_all(bytes);
        }
    }

    impl Drop for LinuxTyper {
        fn drop(&mut self) {
            unsafe {
                libc::ioctl(self.dev.as_raw_fd(), UI_DEV_DESTROY, 0);
            }
        }
    }

    impl Typer for LinuxTyper {
        fn type_text(&mut self, text: &str) -> Result<()> {
            for c in text.chars() {
                let (code, shift) = key_for(c)
                    .ok_or_else(|| format!("no US keycode for character {c:?}"))?;
                if shift {
                    self.emit(EV_KEY, KEY_LEFTSHIFT, 1);
                    self.emit(EV_SYN, SYN_REPORT, 0);
                }
                self.emit(EV_KEY, code, 1);
                self.emit(EV_SYN, SYN_REPORT, 0);
                self.emit(EV_KEY, code, 0);
                self.emit(EV_SYN, SYN_REPORT, 0);
                if shift {
                    self.emit(EV_KEY, KEY_LEFTSHIFT, 0);
                    self.emit(EV_SYN, SYN_REPORT, 0);
                }
            }
            Ok(())
        }
    }

    fn letter_code(c: char) -> u16 {
        match c {
            'a' => 30,
            'b' => 48,
            'c' => 46,
            'd' => 32,
            'e' => 18,
            'f' => 33,
            'g' => 34,
            'h' => 35,
            'i' => 23,
            'j' => 36,
            'k' => 37,
            'l' => 38,
            'm' => 50,
            'n' => 49,
            'o' => 24,
            'p' => 25,
            'q' => 16,
            'r' => 19,
            's' => 31,
            't' => 20,
            'u' => 22,
            'v' => 47,
            'w' => 17,
            'x' => 45,
            'y' => 21,
            'z' => 44,
            _ => 0,
        }
    }

    fn key_for(c: char) -> Option<(u16, bool)> {
        match c {
            '0'..='9' => {
                let d = c as u16 - '0' as u16;
                Some((if d == 0 { 11 } else { d + 1 }, false))
            }
            'a'..='z' => Some((letter_code(c), false)),
            'A'..='Z' => Some((letter_code(c.to_ascii_lowercase()), true)),
            ' ' => Some((57, false)),
            '/' => Some((53, false)),
            '.' => Some((52, false)),
            ',' => Some((51, false)),
            '-' => Some((12, false)),
            '\'' => Some((40, false)),
            '\n' | '\r' => Some((28, false)),
            '\t' => Some((15, false)),
            _ => None,
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::{Result, Typer};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };

    pub struct WindowsTyper;

    fn send_char(c: u16) {
        let down = KEYBDINPUT {
            wVk: 0,
            wScan: c,
            dwFlags: KEYEVENTF_UNICODE,
            time: 0,
            dwExtraInfo: 0,
        };
        let up = KEYBDINPUT {
            wVk: 0,
            wScan: c,
            dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
            time: 0,
            dwExtraInfo: 0,
        };
        let mut di = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: down },
        };
        let mut ui = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: up },
        };
        unsafe {
            SendInput(1, &mut di, std::mem::size_of::<INPUT>() as i32);
            SendInput(1, &mut ui, std::mem::size_of::<INPUT>() as i32);
        }
    }

    impl Typer for WindowsTyper {
        fn type_text(&mut self, text: &str) -> Result<()> {
            for c in text.encode_utf16() {
                send_char(c);
            }
            Ok(())
        }
    }
}

pub fn default_typer() -> Result<Box<dyn Typer>> {
    #[cfg(target_os = "macos")]
    {
        return Ok(Box::new(macos::MacTyper));
    }
    #[cfg(target_os = "linux")]
    {
        return Ok(Box::new(linux::LinuxTyper::new()?));
    }
    #[cfg(target_os = "windows")]
    {
        return Ok(Box::new(windows::WindowsTyper));
    }
    #[allow(unreachable_code)]
    Err("no typer backend for this platform".into())
}

#[cfg(target_os = "macos")]
pub fn backend_is_authorized() -> bool {
    macos::is_trusted()
}

#[cfg(not(target_os = "macos"))]
pub fn backend_is_authorized() -> bool {
    true
}
