use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        GetForegroundWindow, IsIconic, IsWindowVisible, SW_HIDE, SW_MINIMIZE, SW_RESTORE,
        SetForegroundWindow, ShowWindow,
    },
};
pub fn active(hwnd: isize) -> bool {
    unsafe { GetForegroundWindow().0 as isize == hwnd }
}
pub fn visible(hwnd: isize) -> bool {
    unsafe { IsWindowVisible(HWND(hwnd as _)).as_bool() && !IsIconic(HWND(hwnd as _)).as_bool() }
}
pub fn hide(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(HWND(hwnd as _), SW_HIDE);
    }
}
pub fn minimize(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(HWND(hwnd as _), SW_MINIMIZE);
    }
}
pub fn show(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(HWND(hwnd as _), SW_RESTORE);
        let _ = SetForegroundWindow(HWND(hwnd as _));
    }
}
pub fn create() -> crate::replay::Result<TrayIcon> {
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id("show", "Abrir Cut", true, None),
        &MenuItem::with_id("capture", "Pausar / reanudar captura", true, None),
        &MenuItem::with_id("save", "Guardar clip", true, None),
        &MenuItem::with_id("exit", "Salir de Cut", true, None),
    ])?;
    // The app icon embedded by build.rs (resource 1); an orange square if it is missing.
    let icon = Icon::from_resource(1, Some((32, 32))).or_else(|_| {
        let mut pixels = vec![0u8; 32 * 32 * 4];
        for y in 0..32 {
            for x in 0..32 {
                if (6..26).contains(&x) && (6..26).contains(&y) {
                    let i = (y * 32 + x) * 4;
                    pixels[i..i + 4].copy_from_slice(&[255, 91, 20, 255]);
                }
            }
        }
        Icon::from_rgba(pixels, 32, 32)
    })?;
    Ok(TrayIconBuilder::new()
        .with_tooltip("Cut")
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .build()?)
}
pub fn poll(hwnd: isize, controller: &crate::controller::Controller) {
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        use crate::controller::Command as C;
        match event.id().as_ref() {
            "show" => show(hwnd),
            "capture" => controller.send(C::Toggle),
            "save" => controller.send(C::Save),
            "exit" => controller.send(C::Exit),
            _ => {}
        }
    }
}
