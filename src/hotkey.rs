use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

use serde::{Deserialize, Serialize};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

use windows::Win32::{
    Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, SystemInformation::GetTickCount64},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_CONTROL, VK_F9, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
        },
        WindowsAndMessaging::{
            CallNextHookEx, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_UP, MSG,
            SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
        },
    },
};

// Reserved virtual-key value used only in Cut's persisted binding representation.
const NUMPAD_ENTER: u8 = 0x0e;

fn physical_key(virtual_key: u32, scan: u32, extended: bool) -> u8 {
    if extended {
        return if virtual_key == 0x0d {
            NUMPAD_ENTER
        } else {
            virtual_key as u8
        };
    }
    match scan {
        0x52 => 0x60,
        0x4f => 0x61,
        0x50 => 0x62,
        0x51 => 0x63,
        0x4b => 0x64,
        0x4c => 0x65,
        0x4d => 0x66,
        0x47 => 0x67,
        0x48 => 0x68,
        0x49 => 0x69,
        0x53 => 0x6e,
        _ => virtual_key as u8,
    }
}

struct Shortcut {
    binding: Arc<AtomicU32>,
    editing: Arc<AtomicBool>,
    key: KeyPress,
    trigger: Box<dyn Fn()>,
    talk: TalkGate,
    talk_binding: u32,
    talk_generation: u32,
}

static TALK_BINDING: AtomicU32 = AtomicU32::new(0);
static TALK_OPEN: AtomicBool = AtomicBool::new(false);
static TALK_PREVIEW: AtomicBool = AtomicBool::new(false);
static TALK_EDITING: AtomicBool = AtomicBool::new(false);
static TALK_GENERATION: AtomicU32 = AtomicU32::new(0);
static TALK_VIRTUAL_KEY: AtomicU32 = AtomicU32::new(0);
static TALK_KEY_AT: AtomicU64 = AtomicU64::new(0);
static CAPTURED: AtomicU32 = AtomicU32::new(0);

pub fn take_captured() -> Option<Binding> {
    let value = CAPTURED.swap(0, Ordering::SeqCst);
    (value != 0).then(|| Binding::unpack(value))
}

pub fn close_talk() {
    TALK_GENERATION.fetch_add(1, Ordering::SeqCst);
    TALK_OPEN.store(false, Ordering::SeqCst);
    TALK_PREVIEW.store(false, Ordering::SeqCst);
}

/// La prueba usa la misma puerta que el micrófono; nunca se persiste.
pub fn set_talk_preview(open: bool) {
    TALK_PREVIEW.store(
        open && !TALK_EDITING.load(Ordering::SeqCst),
        Ordering::SeqCst,
    );
}

pub fn set_editing(editing: bool) {
    TALK_EDITING.store(editing, Ordering::SeqCst);
    close_talk();
}

pub fn configure_talk(binding: Option<Binding>) {
    let next = binding.map_or(0, Binding::packed);
    if TALK_BINDING.swap(next, Ordering::SeqCst) != next {
        close_talk();
    }
}

pub fn talk_open() -> bool {
    if TALK_EDITING.load(Ordering::SeqCst) {
        return false;
    }
    let binding = TALK_BINDING.load(Ordering::SeqCst);
    if binding == 0 || TALK_PREVIEW.load(Ordering::SeqCst) {
        return true;
    }
    if !TALK_OPEN.load(Ordering::SeqCst) {
        return false;
    }
    // Fuera del hook: un escritorio seguro o una suspensión pueden perder el keyup.
    let key = TALK_VIRTUAL_KEY.load(Ordering::SeqCst);
    let held = key != 0 && unsafe { GetAsyncKeyState(key as i32) < 0 };
    let mut gate = TalkGate { open: true };
    if !gate.validate_hold(held, held_modifiers() == Binding::unpack(binding).modifiers) {
        // Windows actualiza GetAsyncKeyState después de retornar del callback.
        // Mientras se confirma la pulsación, se mantiene el audio cerrado sin
        // invalidar un keydown que todavía está siendo entregado.
        if keydown_pending(
            unsafe { GetTickCount64() },
            TALK_KEY_AT.load(Ordering::SeqCst),
        ) {
            return false;
        }
        close_talk();
        return false;
    }
    true
}

fn keydown_pending(now: u64, pressed_at: u64) -> bool {
    now.saturating_sub(pressed_at) < 20
}

#[derive(Default)]
struct TalkGate {
    open: bool,
}

impl TalkGate {
    fn validate_hold(&mut self, key_held: bool, modifiers_match: bool) -> bool {
        self.open &= key_held && modifiers_match;
        self.open
    }
    fn event(&mut self, binding: Binding, key: u8, released: bool, modifiers: u8, editing: bool) {
        let released_modifier = match key {
            0x11 | 0xa2 | 0xa3 => 1,
            0x12 | 0xa4 | 0xa5 => 2,
            0x10 | 0xa0 | 0xa1 => 4,
            _ => 0,
        };
        if editing
            || modifiers != binding.modifiers
            || (released && (key == binding.key || released_modifier & binding.modifiers != 0))
        {
            self.open = false;
        } else if key == binding.key && !released {
            self.open = true;
        }
    }
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
    pub fn label(self) -> String {
        let key = match self.key {
            0x60..=0x69 => format!("Num {}", self.key - 0x60),
            0x70..=0x87 => format!("F{}", self.key - 0x6f),
            NUMPAD_ENTER => "Num Enter".into(),
            0x6a => "Num ×".into(),
            0x6b => "Num +".into(),
            0x6d => "Num −".into(),
            0x6e => "Num .".into(),
            0x6f => "Num /".into(),
            0x1b => "Esc".into(),
            _ => char::from(self.key).to_string(),
        };
        let mut parts = Vec::new();
        for (flag, name) in [(1, "Ctrl"), (2, "Alt"), (4, "Mayús")] {
            if self.modifiers & flag != 0 {
                parts.push(name.to_string());
            }
        }
        parts.push(key);
        parts.join(" + ")
    }
    pub fn is_escape(self) -> bool {
        self.key == 0x1b
    }
    pub fn valid_talk(self) -> bool {
        self.valid() || (self.modifiers <= 7 && matches!(self.key, 48..=57 | 65..=90))
    }
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

    pub fn valid(self) -> bool {
        self.modifiers <= 7
            && (matches!(self.key, 112..=135 | 0x60..=0x6b | 0x6d..=0x6f | NUMPAD_ENTER)
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
                    talk: TalkGate::default(),
                    talk_binding: 0,
                    talk_generation: 0,
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
            close_talk();
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
                let physical =
                    physical_key(key.vkCode, key.scanCode, key.flags.contains(LLKHF_EXTENDED));
                if shortcut.editing.load(Ordering::Relaxed) {
                    shortcut.talk.open = false;
                    close_talk();
                    if !released && !matches!(physical, 0x10..=0x12 | 0xa0..=0xa5 | 0x5b..=0x5c) {
                        CAPTURED.store(
                            Binding {
                                key: physical,
                                modifiers: held_modifiers(),
                            }
                            .packed(),
                            Ordering::SeqCst,
                        );
                    }
                    return;
                }
                let talk_binding = TALK_BINDING.load(Ordering::SeqCst);
                let generation = TALK_GENERATION.load(Ordering::SeqCst);
                if talk_binding != shortcut.talk_binding || generation != shortcut.talk_generation {
                    shortcut.talk = TalkGate::default();
                    shortcut.talk_binding = talk_binding;
                    shortcut.talk_generation = generation;
                }
                if talk_binding != 0 {
                    if physical == Binding::unpack(talk_binding).key && !released {
                        TALK_VIRTUAL_KEY.store(key.vkCode, Ordering::SeqCst);
                        TALK_KEY_AT.store(unsafe { GetTickCount64() }, Ordering::SeqCst);
                    }
                    shortcut.talk.event(
                        Binding::unpack(talk_binding),
                        physical,
                        released,
                        held_modifiers(),
                        shortcut.editing.load(Ordering::Relaxed),
                    );
                    TALK_OPEN.store(shortcut.talk.open, Ordering::SeqCst);
                }
                if released && shortcut.key.held == Some(physical) {
                    shortcut.key.held = None;
                }
                if physical == binding.key
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
    held_modifiers() == binding.modifiers
}

fn held_modifiers() -> u8 {
    let pressed = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe {
        GetAsyncKeyState(i32::from(key.0)) < 0
    };
    let held = u8::from(pressed(VK_CONTROL))
        | (u8::from(pressed(VK_MENU)) << 1)
        | (u8::from(pressed(VK_SHIFT)) << 2);
    held | if pressed(VK_LWIN) || pressed(VK_RWIN) {
        8
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soltar_en_otro_escritorio_cierra_voz_hasta_una_nueva_pulsacion() {
        let binding = Binding {
            key: 119,
            modifiers: 0,
        };
        let mut gate = TalkGate::default();
        gate.event(binding, 119, false, 0, false);
        assert!(gate.validate_hold(true, true));
        assert!(!gate.validate_hold(false, true));
        gate.event(binding, b'A', false, 0, false);
        assert!(!gate.open);
        gate.event(binding, 119, false, 0, false);
        assert!(gate.validate_hold(true, true));
    }

    #[test]
    fn confirmacion_fisica_tolera_el_callback_pero_no_una_tecla_soltada() {
        assert!(keydown_pending(1000, 1000));
        assert!(keydown_pending(1015, 1000));
        assert!(!keydown_pending(1020, 1000));
        assert!(!keydown_pending(2000, 1000));
    }

    #[test]
    fn prueba_de_voz_abre_audio_y_se_cierra_al_editar() {
        configure_talk(Some(Binding {
            key: 119,
            modifiers: 0,
        }));
        set_editing(false);
        assert!(!talk_open());
        set_talk_preview(true);
        assert!(talk_open());
        set_editing(true);
        assert!(!talk_open());
        set_editing(false);
        assert!(!talk_open());
        set_talk_preview(true);
        close_talk();
        assert!(!talk_open());
        configure_talk(None);
        assert!(talk_open());
    }

    #[test]
    fn pulsar_para_hablar_se_cierra_al_soltar_o_editar() {
        let binding = Binding {
            key: 119,
            modifiers: 1,
        };
        let mut gate = TalkGate::default();
        gate.event(binding, 119, false, 1, false);
        assert!(gate.open);
        gate.event(binding, 65, false, 1, false);
        assert!(gate.open);
        gate.event(binding, 119, true, 1, false);
        assert!(!gate.open);
        gate.event(binding, 119, false, 0, false);
        assert!(!gate.open);
        gate.event(binding, 119, false, 1, false);
        gate.event(binding, 0x11, true, 1, false);
        assert!(!gate.open);
        gate.event(binding, 119, false, 1, true);
        assert!(!gate.open);
    }

    #[test]
    fn numpad_conserva_identidad_con_num_lock_y_enter() {
        for (scan, digit, navigation) in [
            (0x52, 0, 0x2d),
            (0x4f, 1, 0x23),
            (0x50, 2, 0x28),
            (0x51, 3, 0x22),
            (0x4b, 4, 0x25),
            (0x4c, 5, 0x0c),
            (0x4d, 6, 0x27),
            (0x47, 7, 0x24),
            (0x48, 8, 0x26),
            (0x49, 9, 0x21),
        ] {
            assert_eq!(physical_key(0x60 + digit, scan, false), 0x60 + digit as u8);
            assert_eq!(physical_key(navigation, scan, false), 0x60 + digit as u8);
            assert_eq!(physical_key(navigation, scan, true), navigation as u8);
        }
        assert_eq!(physical_key(0x0d, 0x1c, true), NUMPAD_ENTER);
        assert_eq!(physical_key(0x0d, 0x1c, false), 0x0d);
        assert_eq!(physical_key(0x2e, 0x53, false), 0x6e);
        for key in (0x60..=0x6f)
            .filter(|key| *key != 0x6c)
            .chain([NUMPAD_ENTER])
        {
            let binding = Binding { key, modifiers: 0 };
            assert!(binding.valid());
            assert_eq!(Binding::unpack(binding.packed()), binding);
        }
        assert!(
            !Binding {
                key: b'1',
                modifiers: 0
            }
            .valid()
        );
    }

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
