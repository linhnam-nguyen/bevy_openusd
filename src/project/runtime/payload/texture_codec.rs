//! NTEX v2: stable little-endian header and mip-major payload. No native asset serialization.
use super::{RuntimeTextureBlob, RuntimeTextureColorSpace};
use anyhow::{Result, bail, ensure};
use bevy::{
    asset::RenderAssetUsages,
    image::{
        Image, ImageAddressMode, ImageSampler, ImageSamplerBorderColor, ImageSamplerDescriptor,
    },
    render::render_resource::{Extent3d, TextureDimension, TextureFormat, WgpuFeatures},
    render::renderer::RenderDevice,
};
const LIMIT: usize = 256 * 1024 * 1024;
const HEADER: usize = 24;
pub(crate) const MAX_TEXTURE_BLOB_BYTES: usize = LIMIT + HEADER;
#[derive(Clone, Debug)]
pub(crate) struct RuntimeTexturePayload {
    pub(crate) version: u16,
    format: u8,
    wrap: u8,
    width: u32,
    height: u32,
    mips: u32,
    data: Vec<u8>,
}
impl RuntimeTexturePayload {
    fn format_tag(format: TextureFormat) -> Option<u8> {
        Some(match format {
            TextureFormat::Rgba8Unorm => 0,
            TextureFormat::Rgba8UnormSrgb => 1,
            TextureFormat::R8Unorm => 2,
            TextureFormat::Rg8Unorm => 3,
            TextureFormat::Rgba16Float => 4,
            _ => return None,
        })
    }
    fn format(&self) -> Result<TextureFormat> {
        Ok(match self.format {
            0 => TextureFormat::Rgba8Unorm,
            1 => TextureFormat::Rgba8UnormSrgb,
            2 => TextureFormat::R8Unorm,
            3 => TextureFormat::Rg8Unorm,
            4 => TextureFormat::Rgba16Float,
            _ => bail!("unknown texture format"),
        })
    }
    fn expected_len(&self) -> Result<usize> {
        ensure!(
            self.width > 0
                && self.height > 0
                && self.mips > 0
                && self.mips <= 32 - self.width.max(self.height).leading_zeros(),
            "invalid texture extent or mip count"
        );
        let stride = match self.format {
            0 | 1 => 4,
            2 => 1,
            3 => 2,
            4 => 8,
            _ => bail!("unknown texture format"),
        };
        let mut size = 0usize;
        for i in 0..self.mips {
            let w = (self.width >> i).max(1) as usize;
            let h = (self.height >> i).max(1) as usize;
            size = w
                .checked_mul(h)
                .and_then(|n| n.checked_mul(stride))
                .and_then(|n| size.checked_add(n))
                .ok_or_else(|| anyhow::anyhow!("texture length overflow"))?;
            ensure!(size <= LIMIT, "texture payload exceeds limit");
        }
        Ok(size)
    }
    pub(crate) fn from_image(image: &Image, authored_black: [bool; 2]) -> Option<Self> {
        let d = &image.texture_descriptor;
        if d.dimension != TextureDimension::D2
            || d.size.depth_or_array_layers != 1
            || d.sample_count != 1
            || image.texture_view_descriptor.is_some()
        {
            return None;
        }
        let wrap = match &image.sampler {
            ImageSampler::Default => 0,
            ImageSampler::Descriptor(s) => {
                let mode = |m| match m {
                    ImageAddressMode::ClampToEdge => Some(0),
                    ImageAddressMode::Repeat => Some(1),
                    ImageAddressMode::MirrorRepeat => Some(2),
                    ImageAddressMode::ClampToBorder => Some(3),
                    _ => None,
                };
                let mut supported = ImageSamplerDescriptor::linear();
                supported.address_mode_u = s.address_mode_u;
                supported.address_mode_v = s.address_mode_v;
                let u = mode(s.address_mode_u)?;
                let v = mode(s.address_mode_v)?;
                if u == 3 || v == 3 {
                    supported.border_color = Some(ImageSamplerBorderColor::TransparentBlack);
                }
                if *s != supported {
                    return None;
                }
                let [u, v] = std::array::from_fn(|i| if authored_black[i] { 3 } else { [u, v][i] });
                1 + u + 4 * v
            }
        };
        let mut payload = Self {
            version: 2,
            wrap,
            format: Self::format_tag(d.format)?,
            width: d.size.width,
            height: d.size.height,
            mips: d.mip_level_count,
            data: Vec::new(),
        };
        let data = image.data.as_ref()?;
        if payload.expected_len().ok()? != data.len() {
            return None;
        }
        payload.data = data.clone();
        Some(payload)
    }
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.version == 2 && self.expected_len()? == self.data.len(),
            "invalid texture payload"
        );
        let mut b = Vec::with_capacity(HEADER + self.data.len());
        b.extend(b"NTEX");
        b.extend(2u16.to_le_bytes());
        b.push(self.format);
        b.push(self.wrap);
        for n in [self.width, self.height, self.mips, self.data.len() as u32] {
            b.extend(n.to_le_bytes());
        }
        b.extend(&self.data);
        Ok(b)
    }
    pub(crate) fn decode(b: &[u8]) -> Result<Self> {
        ensure!(b.len() <= LIMIT + HEADER, "texture payload exceeds limit");
        if !b.starts_with(b"NTEX") {
            let old: RuntimeTextureBlob = serde_json::from_slice(b)?;
            old.validate()?;
            return Ok(Self {
                version: 1,
                wrap: 0,
                format: match old.color_space {
                    RuntimeTextureColorSpace::Linear => 0,
                    RuntimeTextureColorSpace::Srgb => 1,
                },
                width: old.width,
                height: old.height,
                mips: 1,
                data: old.rgba8,
            });
        }
        ensure!(
            b.len() >= HEADER && b[4..6] == 2u16.to_le_bytes() && b[7] <= 16,
            "unsupported NTEX header"
        );
        let n = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let mut out = Self {
            version: 2,
            wrap: b[7],
            format: b[6],
            width: n(8),
            height: n(12),
            mips: n(16),
            data: Vec::new(),
        };
        ensure!(
            out.expected_len()? == n(20) as usize && b.len() - HEADER == n(20) as usize,
            "invalid NTEX payload length"
        );
        out.data = b[HEADER..].to_vec();
        Ok(out)
    }
    /// Authored Black intent is part of the durable payload, even if the
    /// current device can only realize it as edge clamp.
    pub(crate) fn authored_black_axes(&self) -> [bool; 2] {
        if self.wrap == 0 {
            return [false; 2];
        }
        let packed = self.wrap - 1;
        [packed % 4 == 3, packed / 4 == 3]
    }

    pub(crate) fn validate_device_extent(&self, max_extent: u32) -> Result<()> {
        ensure!(
            self.width <= max_extent && self.height <= max_extent,
            "texture extent exceeds render device limit"
        );
        Ok(())
    }

    pub(crate) fn into_image(self, device: Option<&RenderDevice>) -> Result<Image> {
        if let Some(device) = device {
            self.validate_device_extent(device.limits().max_texture_dimension_2d)?;
        }
        let border_supported = device.is_some_and(|device| {
            device
                .features()
                .contains(WgpuFeatures::ADDRESS_MODE_CLAMP_TO_BORDER)
        });
        ensure!(
            self.expected_len()? == self.data.len(),
            "invalid texture payload"
        );
        let mut image = Image::new_uninit(
            Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            self.format()?,
            RenderAssetUsages::default(),
        );
        image.data = Some(self.data);
        image.texture_descriptor.mip_level_count = self.mips;
        if self.wrap > 0 {
            let mode = |v| match v {
                0 => ImageAddressMode::ClampToEdge,
                1 => ImageAddressMode::Repeat,
                2 => ImageAddressMode::MirrorRepeat,
                3 if border_supported => ImageAddressMode::ClampToBorder,
                _ => ImageAddressMode::ClampToEdge,
            };
            let mut s = ImageSamplerDescriptor::linear();
            let u = (self.wrap - 1) % 4;
            s.address_mode_u = mode(u);
            let v = (self.wrap - 1) / 4;
            s.address_mode_v = mode(v);
            if border_supported && (u == 3 || v == 3) {
                s.border_color = Some(ImageSamplerBorderColor::TransparentBlack);
            }
            image.sampler = ImageSampler::Descriptor(s);
        }
        Ok(image)
    }
}
