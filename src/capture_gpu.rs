use std::time::{Duration, Instant};

use win_native_media::{CaptureTarget, CapturedFrame};
use windows::{
    Graphics::{
        Capture::*,
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
        SizeInt32,
    },
    Win32::{
        Foundation::HWND,
        Graphics::Direct3D11::ID3D11Device,
        System::WinRT::{
            Direct3D11::IDirect3DDxgiInterfaceAccess,
            Graphics::Capture::IGraphicsCaptureItemInterop,
        },
        UI::WindowsAndMessaging::IsWindow,
    },
    core::{Interface, factory},
};
use windows_capture::{
    d3d11,
    dxgi_duplication_api::{DxgiDuplicationApi, Error},
    monitor::Monitor,
};

use crate::{audio::Clock, cursor::Cursor, replay::Result};

pub enum Capture {
    Monitor(Box<DxgiDuplicationApi>),
    Window(Box<WindowCapture>),
}

impl Capture {
    pub fn open(target: CaptureTarget, include_cursor: bool) -> Result<Self> {
        match target {
            CaptureTarget::Monitor(index) => {
                let monitor = Monitor::enumerate()?
                    .get(index)
                    .copied()
                    .ok_or("La pantalla ya no está conectada.")?;
                Ok(Self::Monitor(Box::new(DxgiDuplicationApi::new(monitor)?)))
            }
            CaptureTarget::Window(handle) => Ok(Self::Window(Box::new(WindowCapture::open(
                HWND(handle as *mut _),
                include_cursor,
            )?))),
        }
    }

    pub fn device(&self) -> &ID3D11Device {
        match self {
            Self::Monitor(screen) => screen.device(),
            Self::Window(window) => &window.device,
        }
    }

    pub fn with_frame(
        &mut self,
        clock: &Clock,
        cursor: Option<&mut Cursor>,
        run: impl FnOnce(&CapturedFrame) -> Result<()>,
    ) -> Result<bool> {
        match self {
            Self::Monitor(screen) => match screen.acquire_next_frame(0) {
                Ok(frame) => {
                    if let Some(cursor) = cursor {
                        cursor.update(&frame)?;
                    } else if frame.frame_info().LastPresentTime == 0 {
                        return Ok(false);
                    }
                    run(&CapturedFrame {
                        texture: frame.texture().clone(),
                        width: frame.width(),
                        height: frame.height(),
                        timestamp: clock.counter_time(frame.frame_info().LastPresentTime)?,
                    })?;
                    Ok(true)
                }
                Err(Error::Timeout) => Ok(false),
                Err(error) => Err(error.into()),
            },
            Self::Window(window) => window.with_frame(clock, run),
        }
    }
}

pub struct WindowCapture {
    hwnd: HWND,
    device: ID3D11Device,
    winrt: IDirect3DDevice,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    _item: GraphicsCaptureItem,
    size: SizeInt32,
    last_check: Instant,
}

impl WindowCapture {
    fn open(hwnd: HWND, include_cursor: bool) -> Result<Self> {
        let (device, _) = d3d11::create_d3d_device()?;
        let winrt = d3d11::create_direct3d_device(&device)?;
        let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        session.SetIsCursorCaptureEnabled(include_cursor)?;
        if let Err(error) = session.SetIsBorderRequired(false) {
            let _ = pool.Close();
            return Err(format!(
                "Windows no pudo iniciar la captura de ventana sin borde: {error}"
            )
            .into());
        }
        session.StartCapture()?;
        Ok(Self {
            hwnd,
            device,
            winrt,
            pool,
            session,
            _item: item,
            size,
            last_check: Instant::now(),
        })
    }

    fn with_frame(
        &mut self,
        clock: &Clock,
        run: impl FnOnce(&CapturedFrame) -> Result<()>,
    ) -> Result<bool> {
        let mut frame = match self.pool.TryGetNextFrame() {
            Ok(frame) => frame,
            Err(error) if error.code().0 == 0 => {
                self.wait_for_window()?;
                return Ok(false);
            }
            Err(error) => return Err(error.into()),
        };
        // Evita codificar el fotograma retrasado si el otro búfer ya tiene uno más reciente.
        match self.pool.TryGetNextFrame() {
            Ok(newest) => {
                frame.Close()?;
                frame = newest;
            }
            Err(error) if error.code().0 == 0 => {}
            Err(error) => return Err(error.into()),
        }
        let size = frame.ContentSize()?;
        if size.Width <= 0 || size.Height <= 0 {
            return Ok(false);
        }
        if (size.Width, size.Height) != (self.size.Width, self.size.Height) {
            frame.Close()?;
            self.pool.Recreate(
                &self.winrt,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            )?;
            self.size = size;
            return Ok(false);
        }
        let surface: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let result = run(&CapturedFrame {
            texture: unsafe { surface.GetInterface()? },
            width: size.Width as u32,
            height: size.Height as u32,
            timestamp: clock.hns_time(frame.SystemRelativeTime()?.Duration),
        });
        frame.Close()?;
        result.map(|()| true)
    }

    fn wait_for_window(&mut self) -> Result<()> {
        if self.last_check.elapsed() >= Duration::from_millis(250) {
            self.last_check = Instant::now();
            if !unsafe { IsWindow(Some(self.hwnd)) }.as_bool() {
                return Err("La ventana se cerró.".into());
            }
        }
        Ok(())
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
