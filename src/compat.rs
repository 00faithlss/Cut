//! Windows capabilities used by the recording engine. Build checks are kept
//! separate from device support: a supported API can still be denied or absent
//! on a particular GPU/session.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioMode {
    Applications,
    System,
}

#[derive(Clone, Copy, Debug)]
pub struct Capabilities {
    pub build: u32,
}

impl Capabilities {
    pub fn current() -> std::io::Result<Self> {
        use std::sync::OnceLock;

        static VERSION: OnceLock<Result<Capabilities, i32>> = OnceLock::new();
        let version = VERSION.get_or_init(|| {
            // RtlGetVersion is unaffected by the executable's compatibility
            // manifest, unlike GetVersionEx. OSVERSIONINFOW has this ABI.
            #[repr(C)]
            struct VersionInfo {
                size: u32,
                major: u32,
                minor: u32,
                build: u32,
                platform: u32,
                service_pack: [u16; 128],
            }
            #[link(name = "ntdll")]
            unsafe extern "system" {
                fn RtlGetVersion(info: *mut VersionInfo) -> i32;
            }
            let mut info = VersionInfo {
                size: std::mem::size_of::<VersionInfo>() as u32,
                major: 0,
                minor: 0,
                build: 0,
                platform: 0,
                service_pack: [0; 128],
            };
            // SAFETY: info is a writable, correctly sized OSVERSIONINFOW.
            let status = unsafe { RtlGetVersion(&mut info) };
            if status < 0 {
                Err(status)
            } else {
                Ok(Self { build: info.build })
            }
        });
        version.as_ref().copied().map_err(|status| {
            std::io::Error::other(format!(
                "No se pudo consultar la versión de Windows ({status:#x})."
            ))
        })
    }

    pub fn audio_mode(self) -> AudioMode {
        // AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK starts at build 20348.
        // Windows 10 client builds (including LTSC 2019/2021) use endpoint
        // loopback; Windows 11 can keep individual application tracks.
        if self.build >= 20348 {
            AudioMode::Applications
        } else {
            AudioMode::System
        }
    }

    pub fn check_window_capture(self, _include_cursor: bool) -> Result<(), &'static str> {
        if self.build < 18362 {
            return Err(
                "Capturar una ventana requiere Windows 10 1903 o posterior. En esta versión elige la pantalla completa.",
            );
        }
        // IsBorderRequired se añadió en 20348, no en Windows 10 2004.
        // Nunca iniciar WGC con el borde amarillo como alternativa silenciosa.
        // https://learn.microsoft.com/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired
        if self.build < 20348 {
            return Err(
                "Capturar una ventana sin borde requiere Windows con compilación 20348 o posterior (Windows 11). En Windows 10/LTSC elige la pantalla completa, que usa DXGI sin borde amarillo.",
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_10_uses_system_audio_instead_of_unsupported_process_loopback() {
        for build in [10240, 14393, 17763, 18362, 19041, 19044, 19045, 20347] {
            assert_eq!(
                Capabilities { build }.audio_mode(),
                AudioMode::System,
                "{build}"
            );
        }
        for build in [20348, 22000, 22621, 26100] {
            assert_eq!(
                Capabilities { build }.audio_mode(),
                AudioMode::Applications,
                "{build}"
            );
        }
    }

    #[test]
    fn ltsc_2019_window_capture_has_actionable_monitor_fallback() {
        let error = Capabilities { build: 17763 }
            .check_window_capture(true)
            .unwrap_err();
        assert!(error.contains("1903"));
        assert!(error.contains("pantalla"));
        assert!(
            Capabilities { build: 20348 }
                .check_window_capture(true)
                .is_ok()
        );
    }

    #[test]
    fn old_windows_never_silently_ignores_the_hide_cursor_setting() {
        assert!(
            Capabilities { build: 18362 }
                .check_window_capture(false)
                .is_err()
        );
        assert!(
            Capabilities { build: 18363 }
                .check_window_capture(false)
                .is_err()
        );
        assert!(
            Capabilities { build: 20348 }
                .check_window_capture(false)
                .is_ok()
        );
        assert!(
            Capabilities { build: 22000 }
                .check_window_capture(false)
                .is_ok()
        );
    }

    #[test]
    fn windows_without_borderless_capture_never_starts_a_bordered_window_session() {
        for build in [18362, 18363, 19041, 19044, 19045, 20347] {
            for include_cursor in [true, false] {
                let error = Capabilities { build }
                    .check_window_capture(include_cursor)
                    .expect_err("No se debe iniciar WGC con borde como alternativa silenciosa");
                assert!(error.contains("sin borde"), "{build}: {error}");
                assert!(error.contains("pantalla completa"), "{build}: {error}");
            }
        }
        for build in [20348, 22000, 22621, 26100] {
            for include_cursor in [true, false] {
                assert!(
                    Capabilities { build }
                        .check_window_capture(include_cursor)
                        .is_ok()
                );
            }
        }
    }
}
