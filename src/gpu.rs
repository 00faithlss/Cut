use std::{
    cell::Cell,
    mem::ManuallyDrop,
    sync::OnceLock,
    time::{Duration, Instant},
};
use win_native_media::{
    VideoConfig,
    encoder::{EncodedSample, ParameterSets},
};
use windows::{
    Win32::{
        Graphics::{
            Direct3D11::*,
            Dxgi::{
                Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC},
                IDXGIAdapter1, IDXGIDevice,
            },
        },
        Media::MediaFoundation::*,
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
            Variant::VARIANT,
        },
    },
    core::{GUID, Interface},
};

use crate::replay::Result;

pub fn cpu_config(mut config: VideoConfig) -> VideoConfig {
    if config.height > 1080 {
        config.width = 1920;
        config.height = 1080;
    }
    config.fps = config.fps.min(30);
    config.keyframe_interval = config.fps;
    config.bitrate = config.bitrate.min(20_000_000);
    config
}

pub struct Encoder {
    transform: IMFTransform,
    events: Option<IMFMediaEventGenerator>,
    _manager: Option<IMFDXGIDeviceManager>,
    allocator: Option<IMFVideoSampleAllocatorEx>,
    software: Option<SoftwareSurfaces>,
    context: ID3D11DeviceContext,
    output_info: MFT_OUTPUT_STREAM_INFO,
    requested: u32,
    sample_duration: i64,
    pub parameters: ParameterSets,
}

#[derive(Clone)]
pub struct InputFrame {
    sample: IMFSample,
    pub texture: ID3D11Texture2D,
    pub index: u32,
}

struct SoftwareSurfaces {
    textures: [ID3D11Texture2D; 2],
    staging: ID3D11Texture2D,
    next: Cell<usize>,
    width: u32,
    height: u32,
}

static MEDIA_FOUNDATION: OnceLock<windows::core::Result<()>> = OnceLock::new();

impl Encoder {
    pub fn select(
        device: &ID3D11Device,
        requested: VideoConfig,
        force_software: bool,
    ) -> Result<(Self, VideoConfig, bool)> {
        if !force_software && let Ok(encoder) = Self::new(device, requested) {
            return Ok((encoder, requested, false));
        }
        let config = cpu_config(requested);
        unsafe {
            MEDIA_FOUNDATION
                .get_or_init(|| MFStartup(MF_VERSION, MFSTARTUP_FULL))
                .clone()?;
            let encoder = Self::open_software(device, config).map_err(|error| {
                format!(
                    "No se pudo abrir H.264 por hardware ni el respaldo de Windows por CPU: {error}"
                )
            })?;
            Ok((encoder, config, true))
        }
    }

    pub fn new(device: &ID3D11Device, config: VideoConfig) -> Result<Self> {
        unsafe {
            // La GPU también se usa desde los hilos internos del codificador.
            let context = device.GetImmediateContext()?;
            let multithread: ID3D11Multithread = context.cast()?;
            let _ = multithread.SetMultithreadProtected(true);
            MEDIA_FOUNDATION
                .get_or_init(|| MFStartup(MF_VERSION, MFSTARTUP_FULL))
                .clone()?;
            Self::open(device, config)
        }
    }

    unsafe fn open(device: &ID3D11Device, config: VideoConfig) -> Result<Self> {
        unsafe {
            let transform = hardware_transform(device)?;
            let attributes = transform.GetAttributes()?;
            attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
            let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
            let mut manager = None;
            let mut token = 0;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager = manager.ok_or("Media Foundation no devolvió un administrador D3D11.")?;
            manager.ResetDevice(device, token)?;
            transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)?;
            if let Ok(codec) = transform.cast::<ICodecAPI>() {
                for (key, value) in [
                    (CODECAPI_AVEncCommonMeanBitRate, config.bitrate),
                    (CODECAPI_AVEncMPVGOPSize, config.fps),
                    (CODECAPI_AVEncMPVDefaultBPictureCount, 0),
                ] {
                    let _ = codec.SetValue(&key, &VARIANT::from(value));
                }
                let _ = codec.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
            }
            let output = video_type(&config, &MFVideoFormat_H264)?;
            output.SetUINT32(&MF_MT_AVG_BITRATE, config.bitrate)?;
            output.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
            transform.SetOutputType(0, &output, 0)?;
            let input = video_type(&config, &MFVideoFormat_NV12)?;
            transform.SetInputType(0, &input, 0)?;

            let mut raw = std::ptr::null_mut();
            MFCreateVideoSampleAllocatorEx(&IMFVideoSampleAllocatorEx::IID, &mut raw)?;
            let allocator = IMFVideoSampleAllocatorEx::from_raw(raw);
            allocator.SetDirectXManager(&manager)?;
            let pool = attributes_with_slots(1)?;
            pool.SetUINT32(
                &MF_SA_D3D11_BINDFLAGS,
                (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_VIDEO_ENCODER.0) as u32,
            )?;
            allocator.InitializeSampleAllocatorEx(4, 12, &pool, &input)?;
            let events = transform.cast()?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            let output_info = transform.GetOutputStreamInfo(0)?;
            Ok(Self {
                transform,
                events: Some(events),
                _manager: Some(manager),
                allocator: Some(allocator),
                software: None,
                context: device.GetImmediateContext()?,
                output_info,
                requested: 0,
                sample_duration: 10_000_000 / i64::from(config.fps),
                parameters: ParameterSets::default(),
            })
        }
    }

    unsafe fn open_software(device: &ID3D11Device, config: VideoConfig) -> Result<Self> {
        unsafe {
            // CLSID_CMSH264EncoderMFT, declarado por el SDK en wmcodecdsp.h.
            let transform: IMFTransform = CoCreateInstance(
                &GUID::from_u128(0x6ca50344_051a_4ded_9779_a43305165e35),
                None,
                CLSCTX_INPROC_SERVER,
            )?;
            if let Ok(codec) = transform.cast::<ICodecAPI>() {
                for (key, value) in [
                    (CODECAPI_AVEncCommonMeanBitRate, config.bitrate),
                    (CODECAPI_AVEncMPVGOPSize, config.fps),
                    (CODECAPI_AVEncMPVDefaultBPictureCount, 0),
                    (CODECAPI_AVEncCommonQualityVsSpeed, 0),
                ] {
                    let _ = codec.SetValue(&key, &VARIANT::from(value));
                }
                let _ = codec.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
            }
            let output = video_type(&config, &MFVideoFormat_H264)?;
            output.SetUINT32(&MF_MT_AVG_BITRATE, config.bitrate)?;
            output.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Main.0 as u32)?;
            transform.SetOutputType(0, &output, 0)?;
            transform.SetInputType(0, &video_type(&config, &MFVideoFormat_NV12)?, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            let output_info = transform.GetOutputStreamInfo(0)?;
            let create = |usage, bind, access| -> Result<ID3D11Texture2D> {
                let mut texture = None;
                device.CreateTexture2D(
                    &D3D11_TEXTURE2D_DESC {
                        Width: config.width,
                        Height: config.height,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_NV12,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        Usage: usage,
                        BindFlags: bind,
                        CPUAccessFlags: access,
                        ..Default::default()
                    },
                    None,
                    Some(&mut texture),
                )?;
                texture.ok_or_else(|| "No se pudo reservar una superficie NV12.".into())
            };
            let context = device.GetImmediateContext()?;
            let multithread: ID3D11Multithread = context.cast()?;
            let _ = multithread.SetMultithreadProtected(true);
            Ok(Self {
                transform,
                events: None,
                _manager: None,
                allocator: None,
                software: Some(SoftwareSurfaces {
                    textures: [
                        create(D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET.0 as u32, 0)?,
                        create(D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET.0 as u32, 0)?,
                    ],
                    staging: create(D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0 as u32)?,
                    next: Cell::new(0),
                    width: config.width,
                    height: config.height,
                }),
                context,
                output_info,
                requested: 0,
                sample_duration: 10_000_000 / i64::from(config.fps),
                parameters: ParameterSets::default(),
            })
        }
    }

    pub fn finish(&mut self) -> Result<()> {
        unsafe {
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)?;
            self.transform
                .ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)?;
            let deadline = Instant::now() + Duration::from_millis(350);
            let mut output = Vec::new();
            if self.software.is_some() {
                loop {
                    let before = output.len();
                    self.read_packet(&mut output)?;
                    if output.len() == before {
                        break;
                    }
                }
                self.transform
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0)?;
                return Ok(());
            }
            while Instant::now() < deadline {
                match self
                    .events
                    .as_ref()
                    .unwrap()
                    .GetEvent(MF_EVENT_FLAG_NO_WAIT)
                {
                    Ok(event) => {
                        let kind = event.GetType()?;
                        if kind == METransformDrainComplete.0 as u32 {
                            self.transform
                                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0)?;
                            break;
                        }
                        if kind == METransformHaveOutput.0 as u32 {
                            self.read_packet(&mut output)?;
                            output.clear();
                        }
                    }
                    Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }

    pub fn poll(&mut self, packets: &mut Vec<EncodedSample>) -> Result<()> {
        unsafe {
            if self.software.is_some() {
                loop {
                    let before = packets.len();
                    self.read_packet(packets)?;
                    if packets.len() == before {
                        return Ok(());
                    }
                }
            }
            loop {
                let event = match self
                    .events
                    .as_ref()
                    .unwrap()
                    .GetEvent(MF_EVENT_FLAG_NO_WAIT)
                {
                    Ok(event) => event,
                    Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => break,
                    Err(error) => return Err(error.into()),
                };
                let status = event.GetStatus()?;
                status.ok()?;
                let kind = event.GetType()?;
                if kind == METransformNeedInput.0 as u32 {
                    self.requested += 1;
                } else if kind == METransformHaveOutput.0 as u32 {
                    self.read_packet(packets)?;
                }
            }
        }
        Ok(())
    }

    pub fn accepts_input(&self) -> bool {
        if self.software.is_some() {
            unsafe {
                self.transform.GetInputStatus(0).map_or(true, |status| {
                    status & MFT_INPUT_STATUS_ACCEPT_DATA.0 as u32 != 0
                })
            }
        } else {
            self.requested > 0
        }
    }

    pub fn input_frame(&self) -> Result<Option<InputFrame>> {
        unsafe {
            if let Some(software) = &self.software {
                let next = software.next.get();
                software.next.set((next + 1) % 2);
                let sample = MFCreateSample()?;
                sample.AddBuffer(&MFCreateMemoryBuffer(
                    software.width * software.height * 3 / 2,
                )?)?;
                return Ok(Some(InputFrame {
                    sample,
                    texture: software.textures[next].clone(),
                    index: 0,
                }));
            }
            let sample = match self.allocator.as_ref().unwrap().AllocateSample() {
                Ok(sample) => sample,
                Err(error) if error.code() == MF_E_SAMPLEALLOCATOR_EMPTY => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            let buffer = sample.GetBufferByIndex(0)?;
            let planar: IMF2DBuffer = buffer.cast()?;
            buffer.SetCurrentLength(planar.GetContiguousLength()?)?;
            let dxgi: IMFDXGIBuffer = buffer.cast()?;
            let index = dxgi.GetSubresourceIndex()?;
            let mut raw = std::ptr::null_mut();
            dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw)?;
            Ok(Some(InputFrame {
                sample,
                texture: ID3D11Texture2D::from_raw(raw),
                index,
            }))
        }
    }

    pub fn repeat(&self, previous: &InputFrame, frame: &InputFrame) {
        if previous.texture == frame.texture && previous.index == frame.index {
            return;
        }
        unsafe {
            self.context.CopySubresourceRegion(
                &frame.texture,
                frame.index,
                0,
                0,
                0,
                &previous.texture,
                previous.index,
                None,
            );
        }
    }

    pub fn submit(&mut self, frame: &InputFrame, time: Duration) -> Result<()> {
        debug_assert!(self.accepts_input());
        unsafe {
            if let Some(software) = &self.software {
                self.context.CopyResource(&software.staging, &frame.texture);
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                self.context
                    .Map(&software.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                let result = (|| -> Result<()> {
                    let buffer = frame.sample.GetBufferByIndex(0)?;
                    let mut destination = std::ptr::null_mut();
                    buffer.Lock(&mut destination, None, None)?;
                    let row_bytes = software.width as usize;
                    let rows = software.height as usize * 3 / 2;
                    for row in 0..rows {
                        std::ptr::copy_nonoverlapping(
                            (mapped.pData as *const u8).add(row * mapped.RowPitch as usize),
                            destination.add(row * row_bytes),
                            row_bytes,
                        );
                    }
                    buffer.Unlock()?;
                    buffer.SetCurrentLength((rows * row_bytes) as u32)?;
                    Ok(())
                })();
                self.context.Unmap(&software.staging, 0);
                result?;
            }
            frame.sample.SetSampleTime((time.as_nanos() / 100) as i64)?;
            frame.sample.SetSampleDuration(self.sample_duration)?;
            self.transform.ProcessInput(0, &frame.sample, 0)?;
            if self.software.is_none() {
                self.requested -= 1;
            }
        }
        Ok(())
    }

    unsafe fn read_packet(&mut self, packets: &mut Vec<EncodedSample>) -> Result<()> {
        unsafe {
            let info = &self.output_info;
            let mut buffer = MFT_OUTPUT_DATA_BUFFER::default();
            if info.dwFlags
                & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0)
                    as u32
                == 0
            {
                let sample = MFCreateSample()?;
                sample.AddBuffer(&MFCreateMemoryBuffer(info.cbSize.max(4096))?)?;
                buffer.pSample = ManuallyDrop::new(Some(sample));
            }
            let result = self
                .transform
                .ProcessOutput(0, std::slice::from_mut(&mut buffer), &mut 0);
            let sample = ManuallyDrop::take(&mut buffer.pSample);
            ManuallyDrop::drop(&mut buffer.pEvents);
            if let Err(error) = result {
                if error.code() == MF_E_TRANSFORM_STREAM_CHANGE {
                    let output = self.transform.GetOutputAvailableType(0, 0)?;
                    set_color_info(&output)?;
                    self.transform.SetOutputType(0, &output, 0)?;
                    self.output_info = self.transform.GetOutputStreamInfo(0)?;
                    return Ok(());
                }
                if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT {
                    return Ok(());
                }
                return Err(error.into());
            }
            let Some(sample) = sample else {
                return Ok(());
            };
            let timestamp = Duration::from_nanos(sample.GetSampleTime()?.max(0) as u64 * 100);
            let bytes = sample.ConvertToContiguousBuffer()?;
            let mut ptr = std::ptr::null_mut();
            let mut len = 0;
            bytes.Lock(&mut ptr, None, Some(&mut len))?;
            let data = std::slice::from_raw_parts(ptr, len as usize).to_vec();
            bytes.Unlock()?;
            let mut is_keyframe = false;
            for nal in crate::annex_b::units(&data).filter(|nal| !nal.is_empty()) {
                match nal[0] & 31 {
                    5 => is_keyframe = true,
                    7 => self.parameters.sps = nal.to_vec(),
                    8 => self.parameters.pps = nal.to_vec(),
                    _ => {}
                }
            }
            packets.push(EncodedSample {
                data,
                timestamp,
                is_keyframe,
            });
            Ok(())
        }
    }
}

unsafe fn hardware_transform(device: &ID3D11Device) -> Result<IMFTransform> {
    unsafe {
        let dxgi: IDXGIDevice = device.cast()?;
        let adapter: IDXGIAdapter1 = dxgi.GetAdapter()?.cast()?;
        let desc = adapter.GetDesc1()?;
        let luid = desc.AdapterLuid;
        let attributes = attributes_with_slots(1)?;
        attributes.SetBlob(
            &MFT_ENUM_ADAPTER_LUID,
            std::slice::from_raw_parts(
                (&luid as *const windows::Win32::Foundation::LUID).cast(),
                std::mem::size_of_val(&luid),
            ),
        )?;
        let input = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_NV12,
        };
        let output = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_H264,
        };
        let mut list = std::ptr::null_mut();
        let mut count = 0;
        MFTEnum2(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input),
            Some(&output),
            &attributes,
            &mut list,
            &mut count,
        )?;
        if list.is_null() || count == 0 {
            return Err(
                "Este adaptador gráfico no ofrece un codificador H.264 por hardware.".into(),
            );
        }
        let entries = std::slice::from_raw_parts_mut(list, count as usize);
        let mut transform = None;
        for entry in entries {
            if let Some(activate) = entry.take()
                && transform.is_none()
            {
                transform = activate.ActivateObject::<IMFTransform>().ok();
            }
        }
        CoTaskMemFree(Some(list.cast()));
        transform.ok_or_else(|| "No se pudo abrir el codificador H.264 por hardware.".into())
    }
}

unsafe fn attributes_with_slots(slots: u32) -> Result<IMFAttributes> {
    unsafe {
        let mut result = None;
        MFCreateAttributes(&mut result, slots)?;
        result.ok_or_else(|| "Media Foundation no devolvió sus atributos.".into())
    }
}

unsafe fn video_type(config: &VideoConfig, subtype: &GUID) -> Result<IMFMediaType> {
    unsafe {
        let format = MFCreateMediaType()?;
        let pair = |first: u32, second: u32| (u64::from(first) << 32) | u64::from(second);
        format.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        format.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        format.SetUINT64(&MF_MT_FRAME_SIZE, pair(config.width, config.height))?;
        format.SetUINT64(&MF_MT_FRAME_RATE, pair(config.fps, 1))?;
        format.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pair(1, 1))?;
        format.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        set_color_info(&format)?;
        Ok(format)
    }
}

unsafe fn set_color_info(format: &IMFMediaType) -> windows::core::Result<()> {
    unsafe {
        format.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
        format.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
        format.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        format.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respaldo_cpu_limita_1080p30_sin_escalar_720p_hacia_arriba() {
        let requested = VideoConfig {
            width: 3840,
            height: 2160,
            fps: 120,
            bitrate: 80_000_000,
            keyframe_interval: 120,
        };
        let limited = cpu_config(requested);
        assert_eq!(
            (limited.width, limited.height, limited.fps),
            (1920, 1080, 30)
        );
        assert_eq!(limited.keyframe_interval, 30);
        assert!(limited.bitrate <= 20_000_000);
        let small = cpu_config(VideoConfig {
            width: 1280,
            height: 720,
            fps: 30,
            bitrate: 10_000_000,
            keyframe_interval: 30,
        });
        assert_eq!(
            (small.width, small.height, small.fps, small.bitrate),
            (1280, 720, 30, 10_000_000)
        );
    }
}
