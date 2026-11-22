use std::{cell::RefCell, sync::mpsc, thread};

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
    key: F9,
    trigger: Box<dyn Fn()>,
}

thread_local! {
    static SHORTCUT: RefCell<Option<Shortcut>> = const { RefCell::new(None) };
}

pub fn listen(trigger: impl Fn() + Send + 'static) -> bool {
    let (ready, registered) = mpsc::sync_channel(1);
    if thread::Builder::new()
        .name("cut-hotkey".into())
        .spawn(move || {
            SHORTCUT.with(|shortcut| {
                shortcut.replace(Some(Shortcut {
                    key: F9::default(),
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
        if key.vkCode == u32::from(VK_F9.0) {
            SHORTCUT.with_borrow_mut(|shortcut| {
                if let Some(shortcut) = shortcut {
                    let released = key.flags.contains(LLKHF_UP);
                    let modified = !released
                        && !shortcut.key.held
                        && [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
                            .into_iter()
                            .any(|modifier| unsafe { GetAsyncKeyState(i32::from(modifier.0)) < 0 });
                    if shortcut.key.press(released, modified) {
                        (shortcut.trigger)();
                    }
                }
            });
        }
    }
    unsafe { CallNextHookEx(None, code, message, data) }
}

#[derive(Default)]
struct F9 {
    held: bool,
}

impl F9 {
    fn press(&mut self, released: bool, modified: bool) -> bool {
        let was_held = std::mem::replace(&mut self.held, !released);
        !was_held && !released && !modified
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f9_guarda_una_vez_por_pulsacion_sin_modificadores() {
        let mut key = F9::default();
        assert!(key.press(false, false));
        assert!(!key.press(false, false));
        assert!(!key.press(true, false));
        assert!(!key.press(false, true));
        assert!(!key.press(false, false));
        assert!(!key.press(true, false));
        assert!(key.press(false, false));
    }
}
