use std::marker::PhantomData;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

// COM debe cerrarse en el mismo hilo que lo inició.
pub struct Mta(PhantomData<*mut ()>);

impl Mta {
    pub fn enter() -> windows::core::Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Self(PhantomData))
    }
}

impl Drop for Mta {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::COINIT_APARTMENTTHREADED;

    #[test]
    fn nested_guards_release_com_on_error() {
        std::thread::spawn(|| {
            let exit_early = || -> Result<(), &'static str> {
                let _first = Mta::enter().unwrap();
                let _nested = Mta::enter().unwrap();
                Err("interrumpido")
            };
            assert!(exit_early().is_err());
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
                CoUninitialize();
            }
        })
        .join()
        .unwrap();
    }
}
