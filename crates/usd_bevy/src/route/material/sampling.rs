//! CPU mip generation for source RGBA8 images. Base pixels remain unchanged.
use bevy::image::{
    Image, ImageAddressMode, ImageSampler, ImageSamplerBorderColor, ImageSamplerDescriptor,
};
use bevy::render::render_resource::{TextureDimension, TextureFormat};

const MAX_BYTES: usize = 256 * 1024 * 1024;
fn linear(v: u8) -> f64 {
    let x = f64::from(v) / 255.0;
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}
fn encode(v: f64, srgb: bool) -> u8 {
    let x = v.clamp(0.0, 1.0);
    let x = if !srgb {
        x
    } else if x <= 0.0031308 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (x * 255.0).round() as u8
}

pub(super) fn finalize(image: &mut Image, srgb: bool, normal: bool) {
    let desc = &image.texture_descriptor;
    if desc.mip_level_count != 1
        || desc.dimension != TextureDimension::D2
        || desc.size.depth_or_array_layers != 1
        || !matches!(
            desc.format,
            TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb
        )
    {
        return;
    }
    let (mut w, mut h) = (desc.size.width as usize, desc.size.height as usize);
    let Some(bytes) = image.data.as_mut() else {
        return;
    };
    if w == 0 || h == 0 || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(bytes.len()) {
        return;
    }
    let mut total = bytes.len();
    let (mut tw, mut th) = (w, h);
    while tw > 1 || th > 1 {
        tw = (tw / 2).max(1);
        th = (th / 2).max(1);
        let Some(n) = tw
            .checked_mul(th)
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| total.checked_add(n))
        else {
            return;
        };
        if n > MAX_BYTES {
            return;
        }
        total = n;
    }
    if bytes.try_reserve(total - bytes.len()).is_err() {
        return;
    }
    let mut offset = 0;
    let mut count = 1;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let next_offset = bytes.len();
        for y in 0..nh {
            for x in 0..nw {
                // Exact area weights include odd final rows/columns rather than dropping them.
                let (x0, x1) = (
                    x as f64 * w as f64 / nw as f64,
                    (x + 1) as f64 * w as f64 / nw as f64,
                );
                let (y0, y1) = (
                    y as f64 * h as f64 / nh as f64,
                    (y + 1) as f64 * h as f64 / nh as f64,
                );
                let mut sum = [0.0; 4];
                let mut weight = 0.0;
                for sy in y0.floor() as usize..(y1.ceil() as usize).min(h) {
                    for sx in x0.floor() as usize..(x1.ceil() as usize).min(w) {
                        let k = (x1.min((sx + 1) as f64) - x0.max(sx as f64))
                            * (y1.min((sy + 1) as f64) - y0.max(sy as f64));
                        let p = offset + 4 * (sy * w + sx);
                        for c in 0..4 {
                            let value = if srgb && !normal && c < 3 {
                                linear(bytes[p + c])
                            } else {
                                f64::from(bytes[p + c]) / 255.0
                            };
                            sum[c] += k * value;
                        }
                        weight += k;
                    }
                }
                for v in &mut sum {
                    *v /= weight;
                }
                if normal {
                    let n = [sum[0] * 2.0 - 1.0, sum[1] * 2.0 - 1.0, sum[2] * 2.0 - 1.0];
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    let n = if len > 1e-8 {
                        n.map(|v| v / len)
                    } else {
                        [0.0, 0.0, 1.0]
                    };
                    for c in 0..3 {
                        sum[c] = n[c] * 0.5 + 0.5;
                    }
                }
                bytes.extend((0..4).map(|c| encode(sum[c], srgb && !normal && c < 3)));
            }
        }
        offset = next_offset;
        w = nw;
        h = nh;
        count += 1;
    }
    image.texture_descriptor.mip_level_count = count;
    image.sampler = ImageSampler::linear();
}

// Black falls back to edge clamp on devices without portable border support.
// Do not emit an unsupported GPU sampler descriptor.
pub(super) fn apply_wrap(image: &mut Image, wrap: [u8; 2], border_supported: bool) {
    let mode = |v| match v {
        1 => ImageAddressMode::MirrorRepeat,
        3 if border_supported => ImageAddressMode::ClampToBorder,
        2 | 3 => ImageAddressMode::ClampToEdge,
        _ => ImageAddressMode::Repeat,
    };
    let mut sampler = ImageSamplerDescriptor::linear();
    sampler.address_mode_u = mode(wrap[0]);
    sampler.address_mode_v = mode(wrap[1]);
    if border_supported && wrap.contains(&3) {
        sampler.border_color = Some(ImageSamplerBorderColor::TransparentBlack);
    }
    image.sampler = ImageSampler::Descriptor(sampler);
}
