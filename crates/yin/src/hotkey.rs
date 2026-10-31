//! A system-wide hotkey through Carbon's `RegisterEventHotKey`, which macOS
//! delivers to the app without Accessibility permission, whichever app is in
//! front. macOS doesn't refuse a combination another app registered too, so
//! a clash can't be detected; both apps get the key.

use std::ffi::c_void;

/// The default for bringing up the repository switcher from anywhere.
pub const DEFAULT: &str = "cmd-shift-space";

/// A key and its modifiers, as Carbon numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    key_code: u32,
    modifiers: u32,
}

const CMD: u32 = 0x100;
const SHIFT: u32 = 0x200;
const OPTION: u32 = 0x800;
const CONTROL: u32 = 0x1000;

impl Hotkey {
    /// Parses a binding written like yin's keys, such as `ctrl-alt-y`. At
    /// least one of cmd, ctrl and alt is required, so a plain key can't be
    /// taken from every other app.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut modifiers = 0;
        let mut parts: Vec<&str> = text.split('-').collect();
        // `cmd--` ends in an empty part for the minus key.
        let key = match parts.pop() {
            Some("") => "-",
            Some(key) => key,
            None => return Err(format!("{text:?} has no key")),
        };
        for part in parts.into_iter().filter(|part| !part.is_empty()) {
            modifiers |= match part {
                "cmd" => CMD,
                "shift" => SHIFT,
                "alt" => OPTION,
                "ctrl" => CONTROL,
                other => return Err(format!("unknown modifier {other:?} in {text:?}")),
            };
        }
        if modifiers & (CMD | OPTION | CONTROL) == 0 {
            return Err(format!("{text:?} needs cmd, ctrl or alt"));
        }
        let key_code = key_code(key).ok_or_else(|| format!("unknown key {key:?} in {text:?}"))?;
        Ok(Self {
            key_code,
            modifiers,
        })
    }
}

/// The virtual key code of `key` on an ANSI keyboard.
fn key_code(key: &str) -> Option<u32> {
    const LETTERS: [u32; 26] = [
        0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16,
        6,
    ];
    const DIGITS: [u32; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_lowercase() {
            return Some(LETTERS[(c as u8 - b'a') as usize]);
        }
        if c.is_ascii_digit() {
            return Some(DIGITS[(c as u8 - b'0') as usize]);
        }
    }
    Some(match key {
        "space" => 49,
        "enter" => 36,
        "tab" => 48,
        "escape" => 53,
        "-" => 27,
        "=" => 24,
        "[" => 33,
        "]" => 30,
        ";" => 41,
        "'" => 39,
        "," => 43,
        "." => 47,
        "/" => 44,
        "\\" => 42,
        "`" => 50,
        _ => return None,
    })
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: Handler,
        count: u32,
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        out: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyId,
        target: *mut c_void,
        options: u32,
        out: *mut *mut c_void,
    ) -> i32;
}

const KEYBOARD_EVENTS: u32 = u32::from_be_bytes(*b"keyb");
const HOT_KEY_PRESSED: u32 = 5;

/// Calls `on_press` on the main thread whenever `hotkey` is pressed, in any
/// app. Lasts for the life of the process.
pub fn register(hotkey: Hotkey, on_press: impl Fn() + 'static) -> Result<(), String> {
    extern "C" fn handle(_: *mut c_void, _: *mut c_void, user_data: *mut c_void) -> i32 {
        // SAFETY: `user_data` is the leaked callback below, never freed.
        let on_press = unsafe { &*(user_data as *const Box<dyn Fn()>) };
        on_press();
        0
    }
    let on_press: Box<Box<dyn Fn()>> = Box::new(Box::new(on_press));
    let user_data = Box::into_raw(on_press) as *mut c_void;
    let spec = EventTypeSpec {
        event_class: KEYBOARD_EVENTS,
        event_kind: HOT_KEY_PRESSED,
    };
    // SAFETY: plain Carbon calls on the main thread with valid pointers; the
    // handler and its data live for the rest of the process.
    unsafe {
        let target = GetApplicationEventTarget();
        let status = InstallEventHandler(target, handle, 1, &spec, user_data, std::ptr::null_mut());
        if status != 0 {
            return Err(format!("couldn't listen for the hotkey (error {status})"));
        }
        let mut registered = std::ptr::null_mut();
        let id = EventHotKeyId {
            signature: u32::from_be_bytes(*b"yin "),
            id: 1,
        };
        let status = RegisterEventHotKey(
            hotkey.key_code,
            hotkey.modifiers,
            id,
            target,
            0,
            &mut registered,
        );
        if status != 0 {
            return Err(format!("couldn't register {hotkey:?} (error {status})"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bindings() {
        assert_eq!(
            Hotkey::parse("ctrl-alt-y"),
            Ok(Hotkey {
                key_code: 16,
                modifiers: CONTROL | OPTION
            })
        );
        assert_eq!(Hotkey::parse("cmd-shift-space").unwrap().key_code, 49);
        assert_eq!(Hotkey::parse("cmd--").unwrap().key_code, 27);
        assert_eq!(Hotkey::parse("alt-0").unwrap().key_code, 29);
        assert!(Hotkey::parse("shift-a").is_err());
        assert!(Hotkey::parse("y").is_err());
        assert!(Hotkey::parse("hyper-y").is_err());
        assert!(Hotkey::parse("ctrl-f13").is_err());
    }
}
