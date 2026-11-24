use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    thread,
};

use serde::{Deserialize, Serialize};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

use windows::Win32::{
    Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_CONTROL, VK_F9, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
        },
        WindowsAndMessaging::{
            CallNextHookEx, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_UP, MSG,
            SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
        },
    },
};

struct Shortcut {
    binding: Arc<AtomicU32>,
    editing: Arc<AtomicBool>,
    key: KeyPress,
    trigger: Box<dyn Fn()>,
}

thread_local! {
    static SHORTCUT: RefCell<Option<Shortcut>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Binding {
    key: u8,
    modifiers: u8,
}

impl Binding {
    const REGISTRY_KEY: &str = r"Software\Cut";
    const VALUE: &str = "ClipHotkey";
    const DEFAULT: Self = Self {
        key: VK_F9.0 as u8,
        modifiers: 0,
    };

    pub fn packed(self) -> u32 {
        u32::from(self.key) | (u32::from(self.modifiers) << 8)
    }

    fn unpack(value: u32) -> Self {
        Self {
            key: value as u8,
            modifiers: (value >> 8) as u8,
        }
    }

    fn valid(self) -> bool {
        self.modifiers <= 7
            && (matches!(self.key, 112..=135)
                || (self.modifiers & 3 != 0 && matches!(self.key, 48..=57 | 65..=90)))
    }

    pub fn load() -> Self {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(Self::REGISTRY_KEY)
            .and_then(|key| key.get_value::<u32, _>(Self::VALUE))
            .ok()
            .filter(|value| *value <= u32::from(u16::MAX))
            .map(Self::unpack)
            .filter(|binding| binding.valid())
            .unwrap_or(Self::DEFAULT)
    }

    pub fn save(self) -> std::io::Result<()> {
        if !self.valid() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Las letras y números requieren Ctrl o Alt.",
            ));
        }
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(Self::REGISTRY_KEY)?;
        key.set_value(Self::VALUE, &self.packed())
    }
}

pub fn listen(
    binding: Arc<AtomicU32>,
    editing: Arc<AtomicBool>,
    trigger: impl Fn() + Send + 'static,
) -> bool {
    let (ready, registered) = mpsc::sync_channel(1);
    if thread::Builder::new()
        .name("cut-hotkey".into())
        .spawn(move || {
            SHORTCUT.with(|shortcut| {
                shortcut.replace(Some(Shortcut {
                    binding,
                    editing,
                    key: KeyPress::default(),
                    trigger: Box::new(trigger),
                }));
            });
            let hook = unsafe {
                GetModuleHandleW(None).and_then(|module| {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), Some(HINSTANCE(module.0)), 0)
                })
            };
            let _ = ready.send(hook.is_ok());
            if let Ok(hook) = hook {
                // Windows entrega la llamada en este hilo, independiente de la interfaz.
                let mut message = MSG::default();
                while unsafe { GetMessageW(&mut message, None, 0, 0).0 } > 0 {}
                let _ = unsafe { UnhookWindowsHookEx(hook) };
            }
        })
        .is_err()
    {
        return false;
    }
    registered.recv().unwrap_or(false)
}

unsafe extern "system" fn on_key(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let key = unsafe { &*(data.0 as *const KBDLLHOOKSTRUCT) };
        SHORTCUT.with_borrow_mut(|shortcut| {
            if let Some(shortcut) = shortcut {
                let binding = Binding::unpack(shortcut.binding.load(Ordering::Relaxed));
                let released = key.flags.contains(LLKHF_UP);
                if released && shortcut.key.held.map(u32::from) == Some(key.vkCode) {
                    shortcut.key.held = None;
                }
                if key.vkCode == u32::from(binding.key)
                    && shortcut.key.press(binding.key, released)
                    && !shortcut.editing.load(Ordering::Relaxed)
                    && matches_modifiers(binding)
                {
                    (shortcut.trigger)();
                }
            }
        });
    }
    unsafe { CallNextHookEx(None, code, message, data) }
}

#[derive(Default)]
struct KeyPress {
    held: Option<u8>,
}

impl KeyPress {
    fn press(&mut self, key: u8, released: bool) -> bool {
        let was_held = self.held == Some(key);
        self.held = (!released).then_some(key);
        !was_held && !released
    }
}

fn matches_modifiers(binding: Binding) -> bool {
    let pressed = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe {
        GetAsyncKeyState(i32::from(key.0)) < 0
    };
    let held = u8::from(pressed(VK_CONTROL))
        | (u8::from(pressed(VK_MENU)) << 1)
        | (u8::from(pressed(VK_SHIFT)) << 2);
    held == binding.modifiers && !pressed(VK_LWIN) && !pressed(VK_RWIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atajo_guarda_una_vez_y_admite_cambiar_de_tecla() {
        let mut state = KeyPress::default();
        assert!(state.press(Binding::DEFAULT.key, false));
        assert!(!state.press(Binding::DEFAULT.key, false));
        assert!(state.press(b'X', false));
        assert!(!state.press(b'X', true));
        assert!(state.press(b'X', false));
        assert_eq!(Binding::unpack(Binding::DEFAULT.packed()), Binding::DEFAULT);
        assert!(
            Binding {
                key: b'X',
                modifiers: 1
            }
            .valid()
        );
        assert!(
            !Binding {
                key: b'X',
                modifiers: 0
            }
            .valid()
        );
    }
}
