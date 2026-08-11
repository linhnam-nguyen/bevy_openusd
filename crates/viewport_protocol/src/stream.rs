//! Stream metrics, codec identity, limits, and runtime statistics.

use serde::{Deserialize, Serialize};

use crate::ProtocolValidationError;

pub const MIN_DIMENSION: u32 = 2;
pub const MAX_WIDTH: u32 = 7_680;
pub const MAX_HEIGHT: u32 = 4_320;
pub const MAX_PIXEL_COUNT: u64 = 16_777_216;
pub const MIN_DEVICE_PIXEL_RATIO: f32 = 0.1;
pub const MAX_DEVICE_PIXEL_RATIO: f32 = 4.0;
pub const MIN_FPS: u32 = 1;
pub const MAX_FPS: u32 = 240;

/// Video codec identity negotiated for the viewport stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodecId {
    H264,
    Vp8,
    Vp9,
    Av1,
}

/// Server-side dimensions and rate limits. These are validation metadata, not
/// an instruction to resize the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamLimits {
    pub min_width: u32,
    pub min_height: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixel_count: u64,
    pub min_fps: u32,
    pub max_fps: u32,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            min_width: MIN_DIMENSION,
            min_height: MIN_DIMENSION,
            max_width: MAX_WIDTH,
            max_height: MAX_HEIGHT,
            max_pixel_count: MAX_PIXEL_COUNT,
            min_fps: MIN_FPS,
            max_fps: MAX_FPS,
        }
    }
}

/// Browser-visible measurements and the normalized requested encoded size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewportMetrics {
    pub css_width: u32,
    pub css_height: u32,
    pub device_pixel_ratio: f32,
    pub requested_width: u32,
    pub requested_height: u32,
    pub preferred_fps: Option<u32>,
    pub generation: u64,
}

impl ViewportMetrics {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_dimension("css_width", self.css_width, MAX_WIDTH)?;
        validate_dimension("css_height", self.css_height, MAX_HEIGHT)?;
        validate_encoded_dimension("requested_width", self.requested_width, MAX_WIDTH)?;
        validate_encoded_dimension("requested_height", self.requested_height, MAX_HEIGHT)?;

        if self.device_pixel_ratio.is_nan()
            || !self.device_pixel_ratio.is_finite()
            || !(MIN_DEVICE_PIXEL_RATIO..=MAX_DEVICE_PIXEL_RATIO)
                .contains(&self.device_pixel_ratio)
        {
            return Err(ProtocolValidationError::InvalidDevicePixelRatio {
                value: self.device_pixel_ratio,
            });
        }

        if let Some(fps) = self.preferred_fps
            && !(MIN_FPS..=MAX_FPS).contains(&fps)
        {
            return Err(ProtocolValidationError::InvalidFrameRate { value: fps });
        }

        let pixel_count = u64::from(self.requested_width) * u64::from(self.requested_height);
        if pixel_count > MAX_PIXEL_COUNT {
            return Err(ProtocolValidationError::PixelCountOutOfRange {
                width: self.requested_width,
                height: self.requested_height,
                maximum: MAX_PIXEL_COUNT,
            });
        }

        Ok(())
    }
}

fn validate_dimension(
    field: &'static str,
    value: u32,
    maximum: u32,
) -> Result<(), ProtocolValidationError> {
    if value < MIN_DIMENSION {
        return Err(ProtocolValidationError::InvalidDimension { field, value });
    }
    if value > maximum {
        return Err(ProtocolValidationError::DimensionOutOfRange {
            field,
            value,
            maximum,
        });
    }
    Ok(())
}

fn validate_encoded_dimension(
    field: &'static str,
    value: u32,
    maximum: u32,
) -> Result<(), ProtocolValidationError> {
    validate_dimension(field, value, maximum)?;
    if value % 2 != 0 {
        return Err(ProtocolValidationError::OddEncodedDimension { field, value });
    }
    Ok(())
}

/// Actual active stream configuration reported by the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveStreamConfiguration {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: CodecId,
    pub generation: u64,
}

impl ActiveStreamConfiguration {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_encoded_dimension("width", self.width, MAX_WIDTH)?;
        validate_encoded_dimension("height", self.height, MAX_HEIGHT)?;
        if !(MIN_FPS..=MAX_FPS).contains(&self.fps) {
            return Err(ProtocolValidationError::InvalidFrameRate { value: self.fps });
        }
        if u64::from(self.width) * u64::from(self.height) > MAX_PIXEL_COUNT {
            return Err(ProtocolValidationError::PixelCountOutOfRange {
                width: self.width,
                height: self.height,
                maximum: MAX_PIXEL_COUNT,
            });
        }
        Ok(())
    }
}

/// Optional counters for diagnostics and a future stream status view.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StreamStatistics {
    pub frames_sent: u64,
    pub frames_dropped: u64,
    pub encoded_fps: f32,
    pub bytes_sent: u64,
    pub generation: u64,
}

