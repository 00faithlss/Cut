use std::{
    mem::ManuallyDrop,
    sync::OnceLock,
    time::{Duration, Instant},
};
use win_native_media::{
    VideoConfig,
    convert::Bgra2Nv12,
    encoder::{EncodedSample, ParameterSets, mf_h264::iter_annex_b_nals},
};
use windows::{
    Win32::{
        Graphics::{
            Direct3D11::*,
            Dxgi::{IDXGIAdapter1, IDXGIDevice},
        },
        Media::MediaFoundation::*,
        System::{Com::CoTaskMemFree, Variant::VARIANT},
    },
    core::{GUID, Interface},
};

use crate::replay::Result;

pub struct Encoder {
    transform: IMFTransform,
    events: IMFMediaEventGenerator,
    _manager: IMFDXGIDeviceManager,
    allocator: IMFVideoSampleAllocatorEx,
    convert: Bgra2Nv12,
    requested: u32,
    sample_duration: i64,
    pub parameters: ParameterSets,
}

static MEDIA_FOUNDATION: OnceLock<windows::core::Result<()>> = OnceLock::new();

impl Encoder {
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
            Ok(Self {
                transform,
                events,
                _manager: manager,
                allocator,
                convert: Bgra2Nv12::new(device, config.width, config.height)?,
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
            while Instant::now() < deadline {
                match self.events.GetEvent(MF_EVENT_FLAG_NO_WAIT) {
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
            loop {
                let event = match self.events.GetEvent(MF_EVENT_FLAG_NO_WAIT) {
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

    pub fn feed(&mut self, texture: &ID3D11Texture2D, time: Duration) -> Result<()> {
        if self.requested == 0 {
            return Ok(());
        }
        unsafe {
            let sample = self.allocator.AllocateSample()?;
            let buffer = sample.GetBufferByIndex(0)?;
            let dxgi: IMFDXGIBuffer = buffer.cast()?;
            let mut raw = std::ptr::null_mut();
            dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw)?;
            let nv12 = ID3D11Texture2D::from_raw(raw);
            self.convert
                .convert_into(texture, &nv12, dxgi.GetSubresourceIndex()?)?;
            let planar: IMF2DBuffer = buffer.cast()?;
            buffer.SetCurrentLength(planar.GetContiguousLength()?)?;
            sample.SetSampleTime((time.as_nanos() / 100) as i64)?;
            sample.SetSampleDuration(self.sample_duration)?;
            self.transform.ProcessInput(0, &sample, 0)?;
            self.requested -= 1;
        }
        Ok(())
    }

    unsafe fn read_packet(&mut self, packets: &mut Vec<EncodedSample>) -> Result<()> {
        unsafe {
            let info = self.transform.GetOutputStreamInfo(0)?;
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
                    self.transform.SetOutputType(
                        0,
                        &self.transform.GetOutputAvailableType(0, 0)?,
                        0,
                    )?;
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
            for nal in iter_annex_b_nals(&data).filter(|nal| !nal.is_empty()) {
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
        Ok(format)
    }
}
