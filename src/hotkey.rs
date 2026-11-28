use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex,
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
            GetAsyncKeyState, MAPVK_VK_TO_CHAR, MapVirtualKeyW, VK_CONTROL, VK_F9, VK_LWIN,
            VK_MENU, VK_RWIN, VK_SHIFT,
        },
        WindowsAndMessaging::{
            CallNextHookEx, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_UP, MSG,
            MSLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL,
            WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_RBUTTONDOWN,
            WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
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
    if !matches!(virtual_key, 0x0c | 0x21..=0x28 | 0x2d..=0x2e | 0x60..=0x69 | 0x6e) {
        return virtual_key as u8;
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
    binding: Arc<Mutex<Binding>>,
    editing: Arc<AtomicBool>,
    save: SaveGate,
    trigger: Box<dyn Fn()>,
    talk: TalkGate,
    talk_binding: Option<Binding>,
    pressed: [bool; 256],
    blocked_talk: [bool; 256],
    virtual_keys: [u8; 256],
    capture_generation: u32,
    chord: CaptureChord,
    swallowed_mouse: u8,
    swallowed_windows: u8,
    talk_generation: u32,
}

static TALK_BINDING: Mutex<Option<Binding>> = Mutex::new(None);
static TALK_KEYS: Mutex<[u8; 256]> = Mutex::new([0; 256]);
static CAPTURE_GENERATION: AtomicU32 = AtomicU32::new(0);
static CAPTURE_BLOCKED: Mutex<[bool; 256]> = Mutex::new([false; 256]);
static TALK_OPEN: AtomicBool = AtomicBool::new(false);
static TALK_PREVIEW: AtomicBool = AtomicBool::new(false);
static TALK_EDITING: AtomicBool = AtomicBool::new(false);
static TALK_GENERATION: AtomicU32 = AtomicU32::new(0);
static TALK_KEY_AT: AtomicU64 = AtomicU64::new(0);
static CAPTURED: Mutex<Option<Binding>> = Mutex::new(None);

pub fn take_captured() -> Option<Binding> {
    CAPTURED.lock().unwrap().take()
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
    if TALK_EDITING.swap(editing, Ordering::SeqCst) == editing {
        return;
    }
    *CAPTURED.lock().unwrap() = None;
    let mut blocked = CAPTURE_BLOCKED.lock().unwrap();
    for key in 1..256 {
        blocked[key] = unsafe { GetAsyncKeyState(key as i32) < 0 };
    }
    CAPTURE_GENERATION.fetch_add(1, Ordering::SeqCst);
    close_talk();
}

pub fn configure_talk(binding: Option<Binding>) {
    let mut current = TALK_BINDING.lock().unwrap();
    if *current != binding {
        *current = binding;
        close_talk();
    }
}

pub fn talk_open() -> bool {
    if TALK_EDITING.load(Ordering::SeqCst) {
        return false;
    }
    let binding = *TALK_BINDING.lock().unwrap();
    if binding.is_none() || TALK_PREVIEW.load(Ordering::SeqCst) {
        return true;
    }
    if !TALK_OPEN.load(Ordering::SeqCst) {
        return false;
    }
    // Fuera del hook: un escritorio seguro o una suspensión pueden perder el keyup.
    let binding = binding.unwrap();
    let keys = TALK_KEYS.lock().unwrap();
    let held = binding.keys().all(|key| {
        let vk = keys[key as usize];
        vk != 0 && unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
    });
    let mut gate = TalkGate {
        open: true,
        ..TalkGate::default()
    };
    if !gate.validate_hold(
        held,
        held_modifiers() & binding.modifiers == binding.modifiers,
    ) {
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

struct TalkGate {
    open: bool,
    pressed: [bool; 256],
}
impl Default for TalkGate {
    fn default() -> Self {
        Self {
            open: false,
            pressed: [false; 256],
        }
    }
}

impl TalkGate {
    fn validate_hold(&mut self, key_held: bool, modifiers_match: bool) -> bool {
        self.open &= key_held && modifiers_match;
        self.open
    }
    /// Abre al completar la combinación y cierra al soltar cualquier entrada necesaria.
    /// Otras teclas y modificadores adicionales permiten seguir jugando al hablar.
    fn event(&mut self, binding: Binding, key: u8, released: bool, held: u8, editing: bool) {
        self.pressed[key as usize] = !released;
        if editing {
            self.open = false;
        } else if binding.contains(key) || modifier_bit(key) & binding.modifiers != 0 {
            self.open = (!released || !binding.contains(key))
                && binding.keys().all(|key| self.pressed[key as usize])
                && held & binding.modifiers == binding.modifiers;
        }
    }
}

/// The modifier flag a key stands for (1 Ctrl, 2 Alt, 4 Mayús, 8 Windows), or 0.
fn modifier_bit(key: u8) -> u8 {
    match key {
        0x11 | 0xa2 | 0xa3 => 1,
        0x12 | 0xa4 | 0xa5 => 2,
        0x10 | 0xa0 | 0xa1 => 4,
        0x5b | 0x5c => 8,
        _ => 0,
    }
}

/// Modifier flags that holding `key` itself produces. Alt Gr also reports Ctrl.
fn own_modifiers(key: u8) -> u8 {
    if key == 0xa5 {
        2 | 1
    } else {
        modifier_bit(key)
    }
}

thread_local! {
    static SHORTCUT: RefCell<Option<Shortcut>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Binding {
    key: u8,
    modifiers: u8,
    #[serde(default, skip_serializing_if = "empty_keys")]
    extra_keys: [u64; 4],
}

fn empty_keys(keys: &[u64; 4]) -> bool {
    *keys == [0; 4]
}

impl Binding {
    fn keys(self) -> impl Iterator<Item = u8> {
        let mut keys = self.extra_keys;
        keys[usize::from(self.key) / 64] |= 1u64 << (self.key % 64);
        keys.into_iter().enumerate().flat_map(|(word, mut bits)| {
            std::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let key = word * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                Some(key as u8)
            })
        })
    }
    fn contains(self, key: u8) -> bool {
        self.key == key || self.extra_keys[usize::from(key) / 64] & (1u64 << (key % 64)) != 0
    }
    fn required_modifiers(self) -> u8 {
        self.keys()
            .fold(self.modifiers, |flags, key| flags | own_modifiers(key))
    }

    pub fn label(self) -> String {
        let mut parts = Vec::new();
        for (flag, name) in [(1, "Ctrl"), (2, "Alt"), (4, "Mayús"), (8, "Windows")] {
            if self.modifiers & flag != 0 {
                parts.push(name.to_string());
            }
        }
        parts.push(Self::key_label(self.key));
        parts.extend(
            self.keys()
                .filter(|key| *key != self.key)
                .map(Self::key_label),
        );
        parts.join(" + ")
    }

    fn key_label(key: u8) -> String {
        match key {
            0x01 => "Ratón izquierdo".into(),
            0x02 => "Ratón derecho".into(),
            0x04 => "Ratón central".into(),
            0x05 => "Ratón lateral 1".into(),
            0x06 => "Ratón lateral 2".into(),
            0x5b => "Windows izq.".into(),
            0x5c => "Windows der.".into(),
            0x60..=0x69 => format!("Num {}", key - 0x60),
            0x70..=0x87 => format!("F{}", key - 0x6f),
            NUMPAD_ENTER => "Num Enter".into(),
            0x6a => "Num ×".into(),
            0x6b => "Num +".into(),
            0x6d => "Num −".into(),
            0x6e => "Num .".into(),
            0x6f => "Num /".into(),
            0x1b => "Esc".into(),
            0x08 => "Retroceso".into(),
            0x09 => "Tab".into(),
            0x0d => "Enter".into(),
            0x13 => "Pausa".into(),
            0x14 => "Bloq Mayús".into(),
            0x20 => "Espacio".into(),
            0x21 => "Re Pág".into(),
            0x22 => "Av Pág".into(),
            0x23 => "Fin".into(),
            0x24 => "Inicio".into(),
            0x25 => "←".into(),
            0x26 => "↑".into(),
            0x27 => "→".into(),
            0x28 => "↓".into(),
            0x2c => "Impr Pant".into(),
            0x2d => "Insert".into(),
            0x2e => "Supr".into(),
            0x5d => "Menú".into(),
            0x90 => "Bloq Num".into(),
            0x91 => "Bloq Despl".into(),
            0xa0 => "Mayús izq.".into(),
            0xa1 => "Mayús der.".into(),
            0xa2 => "Ctrl izq.".into(),
            0xa3 => "Ctrl der.".into(),
            0xa4 => "Alt".into(),
            0xa5 => "Alt Gr".into(),
            b'0'..=b'9' | b'A'..=b'Z' => char::from(key).to_string(),
            // Punctuation keys depend on the layout (Ñ, ´, +, -…): ask Windows.
            _ => layout_char(key).unwrap_or_else(|| format!("Tecla {}", key)),
        }
    }
    pub fn is_escape(self) -> bool {
        self.contains(0x1b)
    }
    /// Combinaciones de teclado y ratón; Escape se reserva para cancelar la edición.
    pub fn valid_talk(self) -> bool {
        self.modifiers <= 15
            && self.key != 0
            && self.key != 0xff
            && self
                .keys()
                .all(|key| !matches!(key, 0 | 0x07 | 0x1b | 0xff))
    }
    const REGISTRY_KEY: &str = r"Software\Cut";
    const VALUE: &str = "ClipHotkey";
    const DEFAULT: Self = Self {
        key: VK_F9.0 as u8,
        modifiers: 0,
        extra_keys: [0; 4],
    };

    fn packed(self) -> u32 {
        u32::from(self.key) | (u32::from(self.modifiers) << 8)
    }

    fn unpack(value: u32) -> Self {
        Self {
            key: value as u8,
            modifiers: (value >> 8) as u8,
            extra_keys: [0; 4],
        }
    }

    pub fn valid(self) -> bool {
        self.valid_talk()
    }

    pub fn load() -> Self {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(Self::REGISTRY_KEY)
            .map(|key| Self::load_from(&key))
            .ok()
            .unwrap_or(Self::DEFAULT)
    }

    fn load_from(key: &RegKey) -> Self {
        key.get_value::<String, _>(Self::VALUE)
            .ok()
            .and_then(|value| serde_json::from_str::<Self>(&value).ok())
            .or_else(|| {
                key.get_value::<u32, _>(Self::VALUE)
                    .ok()
                    .filter(|value| *value <= u32::from(u16::MAX))
                    .map(Self::unpack)
            })
            .filter(|binding| binding.valid())
            .unwrap_or(Self::DEFAULT)
    }

    pub fn save(self) -> std::io::Result<()> {
        if !self.valid() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Elige una tecla, un botón del mouse o una combinación válida.",
            ));
        }
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(Self::REGISTRY_KEY)?;
        self.save_to(&key)
    }

    fn save_to(self, key: &RegKey) -> std::io::Result<()> {
        if empty_keys(&self.extra_keys) {
            key.set_value(Self::VALUE, &self.packed())
        } else {
            key.set_value(Self::VALUE, &serde_json::to_string(&self)?)
        }
    }
}

pub fn listen(
    binding: Arc<Mutex<Binding>>,
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
                    save: SaveGate::default(),
                    trigger: Box::new(trigger),
                    talk: TalkGate::default(),
                    talk_binding: None,
                    pressed: [false; 256],
                    blocked_talk: [false; 256],
                    virtual_keys: [0; 256],
                    capture_generation: 0,
                    chord: CaptureChord::default(),
                    swallowed_mouse: 0,
                    swallowed_windows: 0,
                    talk_generation: 0,
                }));
            });
            let hooks = unsafe {
                GetModuleHandleW(None).and_then(|module| {
                    let keyboard = SetWindowsHookExW(
                        WH_KEYBOARD_LL,
                        Some(on_key),
                        Some(HINSTANCE(module.0)),
                        0,
                    )?;
                    match SetWindowsHookExW(
                        WH_MOUSE_LL,
                        Some(on_mouse),
                        Some(HINSTANCE(module.0)),
                        0,
                    ) {
                        Ok(mouse) => Ok((keyboard, mouse)),
                        Err(error) => {
                            let _ = UnhookWindowsHookEx(keyboard);
                            Err(error)
                        }
                    }
                })
            };
            let _ = ready.send(hooks.is_ok());
            if let Ok((keyboard, mouse)) = hooks {
                let mut message = MSG::default();
                while unsafe { GetMessageW(&mut message, None, 0, 0).0 } > 0 {}
                let _ = unsafe { UnhookWindowsHookEx(keyboard) };
                let _ = unsafe { UnhookWindowsHookEx(mouse) };
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
        let physical = physical_key(key.vkCode, key.scanCode, key.flags.contains(LLKHF_EXTENDED));
        let released = key.flags.contains(LLKHF_UP);
        // Durante la captura, Windows no abre Inicio ni quita el foco a Cut.
        let swallow = if matches!(physical, 0x5b | 0x5c) {
            SHORTCUT.with_borrow_mut(|state| {
                let Some(state) = state else {
                    return false;
                };
                let bit = 1 << (physical - 0x5b);
                if released {
                    let swallowed = state.swallowed_windows & bit != 0;
                    state.swallowed_windows &= !bit;
                    return swallowed;
                }
                if TALK_EDITING.load(Ordering::SeqCst) || state.editing.load(Ordering::SeqCst) {
                    state.swallowed_windows |= bit;
                    return true;
                }
                false
            })
        } else {
            false
        };
        input_event(physical, key.vkCode as u8, released);
        if swallow {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, message, data) }
}

unsafe extern "system" fn on_mouse(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let mouse = unsafe { &*(data.0 as *const MSLLHOOKSTRUCT) };
        let event = match message.0 as u32 {
            WM_LBUTTONDOWN => Some((1, false)),
            WM_LBUTTONUP => Some((1, true)),
            WM_RBUTTONDOWN => Some((2, false)),
            WM_RBUTTONUP => Some((2, true)),
            WM_MBUTTONDOWN => Some((4, false)),
            WM_MBUTTONUP => Some((4, true)),
            WM_XBUTTONDOWN | WM_XBUTTONUP => Some((
                if mouse.mouseData >> 16 == 1 { 5 } else { 6 },
                message.0 as u32 == WM_XBUTTONUP,
            )),
            _ => None,
        };
        if let Some((key, released)) = event {
            let swallow = SHORTCUT.with_borrow_mut(|state| {
                let Some(state) = state else {
                    return false;
                };
                let bit = 1 << key;
                let was_swallowed = state.swallowed_mouse & bit != 0;
                if released {
                    state.swallowed_mouse &= !bit;
                    return was_swallowed;
                }
                if TALK_EDITING.load(Ordering::SeqCst) || state.editing.load(Ordering::SeqCst) {
                    state.swallowed_mouse |= bit;
                    return true;
                }
                false
            });
            input_event(key, key, released);
            if swallow {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, message, data) }
}

fn input_event(physical: u8, virtual_key: u8, released: bool) {
    SHORTCUT.with_borrow_mut(|shortcut| {
        let Some(shortcut) = shortcut else {
            return;
        };
        // El evento actual manda: Windows actualiza el estado después del callback.
        for key in 1..256 {
            if key != physical as usize
                && shortcut.pressed[key]
                && unsafe { GetAsyncKeyState(i32::from(shortcut.virtual_keys[key])) >= 0 }
            {
                shortcut.pressed[key] = false;
            }
        }
        for key in 1..256 {
            if shortcut.blocked_talk[key]
                && unsafe { GetAsyncKeyState(i32::from(shortcut.virtual_keys[key])) >= 0 }
            {
                shortcut.blocked_talk[key] = false;
            }
        }
        let previously_pressed = shortcut.pressed;
        shortcut.virtual_keys[physical as usize] = virtual_key;
        shortcut.pressed[physical as usize] = !released;
        let generation = CAPTURE_GENERATION.load(Ordering::SeqCst);
        if generation != shortcut.capture_generation {
            shortcut.capture_generation = generation;
            shortcut.chord = CaptureChord::default();
            shortcut.chord.blocked = *CAPTURE_BLOCKED.lock().unwrap();
        }
        if TALK_EDITING.load(Ordering::SeqCst) || shortcut.editing.load(Ordering::Relaxed) {
            TALK_OPEN.store(false, Ordering::SeqCst);
            shortcut.talk.open = false;
            shortcut.save = SaveGate::default();
            if let Some(binding) = shortcut.chord.event(physical, virtual_key, released) {
                *CAPTURED.lock().unwrap() = Some(binding);
            }
            return;
        }
        let talk_binding = *TALK_BINDING.lock().unwrap();
        let generation = TALK_GENERATION.load(Ordering::SeqCst);
        if talk_binding != shortcut.talk_binding || generation != shortcut.talk_generation {
            shortcut.talk.open = false;
            shortcut.talk_binding = talk_binding;
            shortcut.talk_generation = generation;
            for key in 1..256 {
                shortcut.blocked_talk[key] |= previously_pressed[key]
                    && unsafe { GetAsyncKeyState(i32::from(shortcut.virtual_keys[key])) < 0 };
            }
            shortcut.pressed.fill(false);
            shortcut.pressed[physical as usize] =
                !released && !shortcut.blocked_talk[physical as usize];
        }
        if released {
            shortcut.blocked_talk[physical as usize] = false;
        }
        if shortcut.blocked_talk[physical as usize] {
            shortcut.pressed[physical as usize] = false;
        }
        let held = held_modifiers();
        let held = if released {
            let flag = modifier_bit(physical);
            let other_held = [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c]
                .into_iter()
                .any(|key| {
                    key != physical
                        && modifier_bit(key) == flag
                        && unsafe { GetAsyncKeyState(i32::from(key)) < 0 }
                });
            if other_held { held } else { held & !flag }
        } else {
            held | own_modifiers(physical)
        };
        if let Some(binding) = talk_binding {
            let was_open = shortcut.talk.open;
            shortcut.talk.pressed = shortcut.pressed;
            shortcut.talk.event(
                binding,
                physical,
                released || shortcut.blocked_talk[physical as usize],
                held,
                false,
            );
            if shortcut.talk.open && !was_open {
                *TALK_KEYS.lock().unwrap() = shortcut.virtual_keys;
                TALK_KEY_AT.store(unsafe { GetTickCount64() }, Ordering::SeqCst);
            }
            TALK_OPEN.store(shortcut.talk.open, Ordering::SeqCst);
        }
        let binding = *shortcut.binding.lock().unwrap();
        let talking_with = talk_binding
            .filter(|_| shortcut.talk.open)
            .map_or(0, Binding::required_modifiers);
        if shortcut.save.event(
            binding,
            &shortcut.pressed,
            physical,
            released,
            held,
            talking_with,
        ) {
            (shortcut.trigger)();
        }
    });
}

struct CaptureChord {
    blocked: [bool; 256],
    keys: [bool; 256],
    done: bool,
}
impl Default for CaptureChord {
    fn default() -> Self {
        Self {
            blocked: [false; 256],
            keys: [false; 256],
            done: false,
        }
    }
}
impl CaptureChord {
    fn event(&mut self, key: u8, vk: u8, released: bool) -> Option<Binding> {
        if self.done {
            return None;
        }
        if key == 0x1b && !released {
            self.done = true;
            return Some(Binding {
                key,
                modifiers: 0,
                extra_keys: [0; 4],
            });
        }
        if self.blocked[vk as usize] {
            if released {
                self.blocked[vk as usize] = false;
            }
            return None;
        }
        if !released {
            self.keys[key as usize] = true;
            return None;
        }
        if !self.keys[key as usize] {
            return None;
        }
        self.done = true;
        // Alt Gr se recibe como un Ctrl izquierdo sintético seguido de Alt derecho.
        if self.keys[0xa5] {
            self.keys[0xa2] = false;
        }
        let primary = (1..=254u8)
            .find(|key| self.keys[*key as usize] && modifier_bit(*key) == 0)
            .or_else(|| (1..=254u8).find(|key| self.keys[*key as usize]))?;
        let mut binding = Binding {
            key: primary,
            modifiers: 0,
            extra_keys: [0; 4],
        };
        for key in 1..=254u8 {
            if key != primary && self.keys[key as usize] {
                let flag = modifier_bit(key);
                let repeated_modifier = flag != 0
                    && (1..=254u8).any(|other| {
                        other != key && self.keys[other as usize] && modifier_bit(other) == flag
                    });
                if flag != 0 && modifier_bit(primary) == 0 && !repeated_modifier {
                    binding.modifiers |= own_modifiers(key);
                } else {
                    binding.extra_keys[key as usize / 64] |= 1u64 << (key % 64);
                }
            }
        }
        Some(binding)
    }
}

/// The character a punctuation key types on the current keyboard layout.
fn layout_char(key: u8) -> Option<String> {
    let value = unsafe { MapVirtualKeyW(u32::from(key), MAPVK_VK_TO_CHAR) } & 0xffff;
    char::from_u32(value)
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .map(|c| c.to_uppercase().collect())
}

#[derive(Default)]
struct SaveGate {
    binding: Option<Binding>,
    fired: bool,
}

impl SaveGate {
    fn event(
        &mut self,
        binding: Binding,
        pressed: &[bool; 256],
        key: u8,
        released: bool,
        held: u8,
        talking_with: u8,
    ) -> bool {
        if self.binding != Some(binding) {
            self.binding = Some(binding);
            self.fired = false;
        }
        let required = binding.required_modifiers();
        if held & required != required || !binding.keys().all(|key| pressed[key as usize]) {
            self.fired = false;
            return false;
        }
        if released
            || self.fired
            || !(binding.contains(key) || modifier_bit(key) & binding.modifiers != 0)
        {
            return false;
        }
        if held & !(talking_with & !required) != required {
            return false;
        }
        self.fired = true;
        true
    }
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
            extra_keys: [0; 4],
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
    fn captura_alt_alt_gr_y_tecla_windows() {
        for (keys, expected, modifiers) in [
            (&[0xa4][..], 0xa4, 0),
            (&[0xa4, b'T'], b'T', 2),
            (&[0xa2, 0xa5], 0xa5, 0),
            (&[0x5b], 0x5b, 0),
        ] {
            let mut capture = CaptureChord::default();
            for &key in keys {
                assert!(capture.event(key, key, false).is_none());
            }
            let first = keys[0];
            let binding = capture.event(first, first, true).unwrap();
            assert_eq!(binding.key, expected);
            assert_eq!(binding.modifiers, modifiers);
            assert!(binding.valid());
        }
    }

    #[test]
    fn guardar_admite_teclas_solas_y_combinaciones() {
        let ok = |key, modifiers| {
            Binding {
                key,
                modifiers,
                extra_keys: [0; 4],
            }
            .valid()
        };
        assert!(ok(b'S', 1 | 4)); // Ctrl + Mayús + S
        assert!(ok(0x20, 2)); // Alt + Espacio
        assert!(ok(0x70 + 8, 1)); // Ctrl + F9
        assert!(ok(0x60, 4)); // Mayús + Num 0
        assert!(ok(0xc0, 1 | 2)); // Ctrl + Alt + Ñ
        assert!(ok(0x25, 1)); // Ctrl + ←
        assert!(ok(b'S', 0));
        assert!(ok(b'S', 4));
        assert!(ok(0x20, 0));
        assert!(ok(0xa4, 0));
        assert!(!ok(0x1b, 1));
        assert!(ok(b'S', 8));
    }

    #[test]
    fn cualquier_tecla_del_teclado_sirve_para_hablar() {
        for key in [
            0x01,
            0x02,
            0x04,
            0x05,
            0x06,
            0x5b,
            0x5c,
            0xa4,
            0xa5,
            0xa2,
            0xa3,
            0xa0,
            0xa1,
            0x20,
            0x09,
            0x14,
            b'V',
            b'1',
            0xc0,
            0xba,
            0x25,
            0x60,
            NUMPAD_ENTER,
            0x70,
        ] {
            assert!(
                Binding {
                    key,
                    modifiers: 0,
                    extra_keys: [0; 4]
                }
                .valid_talk(),
                "tecla {key:#x}"
            );
        }
        assert!(
            !Binding {
                key: 0x1b,
                modifiers: 0,
                extra_keys: [0; 4]
            }
            .valid_talk()
        );
    }

    #[test]
    fn hablar_con_alt_no_se_corta_por_el_propio_alt_ni_por_otros_modificadores() {
        let alt = Binding {
            key: 0xa4,
            modifiers: 0,
            extra_keys: [0; 4],
        };
        let mut gate = TalkGate::default();
        gate.event(alt, 0xa4, false, 2, false);
        assert!(gate.open);
        // Correr con Mayús o agacharse con Ctrl mientras hablas.
        gate.event(alt, 0xa0, false, 2 | 4, false);
        gate.event(alt, 0xa0, true, 2, false);
        gate.event(alt, 0xa2, false, 2 | 1, false);
        assert!(gate.open);
        assert!(gate.validate_hold(true, true));
        gate.event(alt, 0xa4, true, 0, false);
        assert!(!gate.open);
        // Una tecla sin modificadores tampoco se corta al pulsar Mayús.
        let v = Binding {
            key: b'V',
            modifiers: 0,
            extra_keys: [0; 4],
        };
        let mut gate = TalkGate::default();
        gate.event(v, b'V', false, 4, false);
        assert!(gate.open);
        gate.event(v, 0xa0, true, 0, false);
        assert!(gate.open);
        assert_eq!(own_modifiers(0xa5), 3);
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
            extra_keys: [0; 4],
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
            extra_keys: [0; 4],
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
        gate.event(binding, 0x11, true, 0, false);
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
            let binding = Binding {
                key,
                modifiers: 0,
                extra_keys: [0; 4],
            };
            assert!(binding.valid());
            assert_eq!(Binding::unpack(binding.packed()), binding);
        }
        assert!(
            Binding {
                key: b'1',
                modifiers: 0,
                extra_keys: [0; 4]
            }
            .valid()
        );
    }

    #[test]
    fn atajo_guarda_una_vez_por_combinacion_completa() {
        let mut capture = CaptureChord::default();
        capture.event(5, 5, false);
        capture.event(b'Q', b'Q', false);
        let binding = capture.event(5, 5, true).unwrap();
        let mut gate = SaveGate::default();
        let mut pressed = [false; 256];
        assert!(binding.valid());
        pressed[5] = true;
        assert!(!gate.event(binding, &pressed, 5, false, 0, 0));
        pressed[b'Q' as usize] = true;
        assert!(gate.event(binding, &pressed, b'Q', false, 0, 0));
        assert!(!gate.event(binding, &pressed, b'Q', false, 0, 0));
        pressed[5] = false;
        assert!(!gate.event(binding, &pressed, 5, true, 0, 0));
        pressed[5] = true;
        assert!(gate.event(binding, &pressed, 5, false, 0, 0));
        assert_eq!(Binding::unpack(Binding::DEFAULT.packed()), Binding::DEFAULT);
    }
}
