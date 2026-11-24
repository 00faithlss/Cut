use windows::{
    Win32::Graphics::{
        Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, Fxc::D3DCompile, ID3DBlob, ID3DInclude},
        Direct3D11::*,
        Dxgi::{
            Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
            *,
        },
    },
    core::{PCSTR, s},
};
use windows_capture::dxgi_duplication_api::DxgiDuplicationFrame;

use crate::replay::Result;

struct Shape {
    view: ID3D11ShaderResourceView,
    width: u32,
    height: u32,
    shader: usize,
}

pub struct Cursor {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vertex: ID3D11VertexShader,
    pixels: [ID3D11PixelShader; 3],
    desktop: ID3D11ShaderResourceView,
    target: ID3D11RenderTargetView,
    texture: ID3D11Texture2D,
    shape: Option<Shape>,
    buffer: Vec<u8>,
    position: Option<(i32, i32)>,
    source: (u32, u32),
    output: (u32, u32),
}

impl Cursor {
    pub fn new(
        device: &ID3D11Device,
        desktop: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        unsafe {
            let code = compile(s!("vertex"), s!("vs_5_0"))?;
            let mut vertex = None;
            device.CreateVertexShader(
                bytes(&code),
                None::<&ID3D11ClassLinkage>,
                Some(&mut vertex),
            )?;
            let mut pixels = Vec::new();
            for name in [s!("color"), s!("masked"), s!("monochrome")] {
                let code = compile(name, s!("ps_5_0"))?;
                let mut shader = None;
                device.CreatePixelShader(
                    bytes(&code),
                    None::<&ID3D11ClassLinkage>,
                    Some(&mut shader),
                )?;
                pixels.push(shader.ok_or("Falta el shader del cursor.")?);
            }
            let mut view = None;
            device.CreateShaderResourceView(desktop, None, Some(&mut view))?;
            let texture = texture(
                device,
                width,
                height,
                D3D11_BIND_RENDER_TARGET.0 as u32,
                None,
            )?;
            let mut target = None;
            device.CreateRenderTargetView(&texture, None, Some(&mut target))?;
            Ok(Self {
                device: device.clone(),
                context: device.GetImmediateContext()?,
                vertex: vertex.ok_or("Falta el shader de vértices del cursor.")?,
                pixels: pixels
                    .try_into()
                    .map_err(|_| "Faltan shaders del cursor.")?,
                desktop: view.ok_or("No se pudo leer la textura del escritorio.")?,
                target: target.ok_or("No se pudo preparar la textura del cursor.")?,
                texture,
                shape: None,
                buffer: Vec::new(),
                position: None,
                source: (width, height),
                output: (width, height),
            })
        }
    }

    pub fn update(&mut self, frame: &DxgiDuplicationFrame<'_>) -> Result<()> {
        let info = frame.frame_info();
        self.source = (frame.width(), frame.height());
        if info.LastMouseUpdateTime != 0 {
            let state = info.PointerPosition;
            self.position = state
                .Visible
                .as_bool()
                .then_some((state.Position.x, state.Position.y));
        }
        if info.PointerShapeBufferSize == 0 {
            return Ok(());
        }
        self.buffer.resize(info.PointerShapeBufferSize as usize, 0);
        let mut required = 0;
        let mut geometry = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
        unsafe {
            frame.duplication().GetFramePointerShape(
                info.PointerShapeBufferSize,
                self.buffer.as_mut_ptr().cast(),
                &mut required,
                &mut geometry,
            )?;
        }
        let monochrome = geometry.Type == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME.0 as u32;
        let (width, pitch) = (geometry.Width, geometry.Pitch as usize);
        let height = if monochrome {
            geometry.Height / 2
        } else {
            geometry.Height
        };
        let needed_pitch = if monochrome {
            width.div_ceil(8)
        } else {
            width.saturating_mul(4)
        };
        if width == 0
            || height == 0
            || pitch < needed_pitch as usize
            || pitch.saturating_mul(geometry.Height as usize) > required as usize
            || required as usize > self.buffer.len()
        {
            return Err("Windows devolvió una forma de cursor inválida.".into());
        }
        let mut mask = Vec::new();
        let (data, row_bytes, shader) = if monochrome {
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let bit = 0x80 >> (x % 8);
                    let at = y * pitch + x / 8;
                    let and = u8::from(self.buffer[at] & bit != 0) * 255;
                    let xor = u8::from(self.buffer[at + height as usize * pitch] & bit != 0) * 255;
                    mask.extend_from_slice(&[0, xor, and, 255]);
                }
            }
            (mask.as_slice(), width * 4, 2)
        } else {
            let masked = geometry.Type == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR.0 as u32;
            (self.buffer.as_slice(), geometry.Pitch, usize::from(masked))
        };
        let texture = texture(
            &self.device,
            width,
            height,
            D3D11_BIND_SHADER_RESOURCE.0 as u32,
            Some(&D3D11_SUBRESOURCE_DATA {
                pSysMem: data.as_ptr().cast(),
                SysMemPitch: row_bytes,
                SysMemSlicePitch: 0,
            }),
        )?;
        let mut view = None;
        unsafe {
            self.device
                .CreateShaderResourceView(&texture, None, Some(&mut view))?;
        }
        self.shape = Some(Shape {
            view: view.ok_or("Falta la textura del puntero.")?,
            width,
            height,
            shader,
        });
        Ok(())
    }

    pub fn render<'a>(&'a self, desktop: &'a ID3D11Texture2D) -> &'a ID3D11Texture2D {
        let (Some((x, y)), Some(shape)) = (self.position, &self.shape) else {
            return desktop;
        };
        let scale_x = self.output.0 as f32 / self.source.0 as f32;
        let scale_y = self.output.1 as f32 / self.source.1 as f32;
        let viewport = D3D11_VIEWPORT {
            TopLeftX: x as f32 * scale_x,
            TopLeftY: y as f32 * scale_y,
            Width: shape.width as f32 * scale_x,
            Height: shape.height as f32 * scale_y,
            MinDepth: 0.,
            MaxDepth: 1.,
        };
        unsafe {
            self.context.CopyResource(&self.texture, desktop);
            self.context.OMSetRenderTargets(
                Some(&[Some(self.target.clone())]),
                None::<&ID3D11DepthStencilView>,
            );
            self.context.RSSetViewports(Some(&[viewport]));
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vertex, None);
            self.context.PSSetShader(&self.pixels[shape.shader], None);
            self.context.PSSetShaderResources(
                0,
                Some(&[Some(self.desktop.clone()), Some(shape.view.clone())]),
            );
            self.context.Draw(3, 0);
            self.context.PSSetShaderResources(0, Some(&[None, None]));
            self.context
                .OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
        }
        &self.texture
    }
}

fn texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    bind: u32,
    source: Option<&D3D11_SUBRESOURCE_DATA>,
) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind,
        ..Default::default()
    };
    let mut result = None;
    unsafe {
        device.CreateTexture2D(&desc, source.map(std::ptr::from_ref), Some(&mut result))?;
    }
    result.ok_or_else(|| "No se pudo crear la textura del cursor.".into())
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let text = include_bytes!("cursor.hlsl");
    let mut blob = None;
    let mut errors = None;
    let result = unsafe {
        D3DCompile(
            text.as_ptr().cast(),
            text.len(),
            PCSTR::null(),
            None,
            None::<&ID3DInclude>,
            entry,
            target,
            0,
            0,
            &mut blob,
            Some(&mut errors),
        )
    };
    if let Err(error) = result {
        return Err(errors
            .map_or_else(
                || error.to_string(),
                |info| String::from_utf8_lossy(bytes(&info)).into_owned(),
            )
            .into());
    }
    blob.ok_or_else(|| "El compilador no devolvió el shader.".into())
}

fn bytes(blob: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize()) }
}
