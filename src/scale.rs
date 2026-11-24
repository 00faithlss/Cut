use std::mem::ManuallyDrop;

use win_native_media::CapturedFrame;
use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::{
            Direct3D11::*,
            Dxgi::Common::{DXGI_FORMAT, DXGI_RATIONAL, DXGI_SAMPLE_DESC},
        },
    },
    core::{Interface, Result},
};

pub struct Scaler {
    gpu: ID3D11Device,
    device: ID3D11VideoDevice,
    context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    outputs: Vec<(ID3D11Texture2D, u32, ID3D11VideoProcessorOutputView)>,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
}

impl Scaler {
    pub fn new(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        fps: u32,
        format: DXGI_FORMAT,
    ) -> Result<Self> {
        unsafe {
            let video: ID3D11VideoDevice = device.cast()?;
            let context: ID3D11VideoContext = device.GetImmediateContext()?.cast()?;
            let rate = DXGI_RATIONAL {
                Numerator: fps,
                Denominator: 1,
            };
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: rate,
                InputWidth: width,
                InputHeight: height,
                OutputFrameRate: rate,
                OutputWidth: width,
                OutputHeight: height,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };
            let enumerator = video.CreateVideoProcessorEnumerator(&desc)?;
            let processor = video.CreateVideoProcessor(&enumerator, 0)?;
            let destination = rect(width, height);
            context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&destination));
            context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&destination));
            context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            Ok(Self {
                gpu: device.clone(),
                device: video,
                context,
                enumerator,
                processor,
                outputs: Vec::new(),
                width,
                height,
                format,
            })
        }
    }

    pub fn create_target(&self) -> Result<ID3D11Texture2D> {
        unsafe {
            let mut texture = None;
            self.gpu.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: self.width,
                    Height: self.height,
                    ArraySize: 1,
                    MipLevels: 1,
                    Format: self.format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                    ..Default::default()
                },
                None,
                Some(&mut texture),
            )?;
            Ok(texture.unwrap())
        }
    }

    fn output_view(
        &mut self,
        target: &ID3D11Texture2D,
        index: u32,
    ) -> Result<ID3D11VideoProcessorOutputView> {
        if let Some((_, _, view)) = self
            .outputs
            .iter()
            .find(|(resource, slice, _)| resource == target && *slice == index)
        {
            return Ok(view.clone());
        }
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            target.GetDesc(&mut desc);
            let view_desc = if desc.ArraySize > 1 {
                D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                    ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2DARRAY,
                    Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                        Texture2DArray: D3D11_TEX2D_ARRAY_VPOV {
                            MipSlice: 0,
                            FirstArraySlice: index,
                            ArraySize: 1,
                        },
                    },
                }
            } else {
                D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                    ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                    ..Default::default()
                }
            };
            let mut view = None;
            self.device.CreateVideoProcessorOutputView(
                target,
                &self.enumerator,
                &view_desc,
                Some(&mut view),
            )?;
            let view = view.unwrap();
            if self.outputs.len() == 12 {
                self.outputs.remove(0);
            }
            self.outputs.push((target.clone(), index, view.clone()));
            Ok(view)
        }
    }

    pub fn stretch(
        &mut self,
        frame: &CapturedFrame,
        target: &ID3D11Texture2D,
        index: u32,
    ) -> Result<()> {
        self.stretch_texture(&frame.texture, frame.width, frame.height, target, index)
    }

    pub fn stretch_texture(
        &mut self,
        texture: &ID3D11Texture2D,
        width: u32,
        height: u32,
        target: &ID3D11Texture2D,
        index: u32,
    ) -> Result<()> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            texture.GetDesc(&mut desc);
        }
        if (width, height) == (self.width, self.height)
            && (desc.Width, desc.Height) == (self.width, self.height)
            && desc.Format == self.format
        {
            unsafe {
                let context: ID3D11DeviceContext = self.context.cast()?;
                context.CopySubresourceRegion(target, index, 0, 0, 0, texture, 0, None);
            }
            return Ok(());
        }
        unsafe {
            let output = self.output_view(target, index)?;
            let mut input = None;
            self.device.CreateVideoProcessorInputView(
                texture,
                &self.enumerator,
                &D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                    ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                    ..Default::default()
                },
                Some(&mut input),
            )?;
            let source = rect(width.min(desc.Width), height.min(desc.Height));
            self.context
                .VideoProcessorSetStreamSourceRect(&self.processor, 0, true, Some(&source));
            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(input),
                ..Default::default()
            };
            let result = self.context.VideoProcessorBlt(
                &self.processor,
                &output,
                0,
                std::slice::from_ref(&stream),
            );
            ManuallyDrop::drop(&mut stream.pInputSurface);
            result?;
        }
        Ok(())
    }
}

fn rect(width: u32, height: u32) -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: width as i32,
        bottom: height as i32,
    }
}
