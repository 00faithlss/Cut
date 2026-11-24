use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
        System::Threading::{CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject},
    },
    core::w,
};

pub struct Instance {
    event: HANDLE,
    mutex: HANDLE,
}

impl Instance {
    pub fn open() -> windows::core::Result<Option<Self>> {
        unsafe {
            let event = CreateEventW(None, false, false, w!("Local\\CutClipper.Abrir"))?;
            let mutex = match CreateMutexW(None, false, w!("Local\\CutClipper.Instancia")) {
                Ok(mutex) => mutex,
                Err(error) => {
                    let _ = CloseHandle(event);
                    return Err(error);
                }
            };
            let exists = GetLastError() == ERROR_ALREADY_EXISTS;
            let instance = Self { event, mutex };
            if exists {
                SetEvent(instance.event)?;
                Ok(None)
            } else {
                Ok(Some(instance))
            }
        }
    }

    pub fn take_open_request(&self) -> bool {
        unsafe { WaitForSingleObject(self.event, 0) == WAIT_OBJECT_0 }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.mutex);
            let _ = CloseHandle(self.event);
        }
    }
}
