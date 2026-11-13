//! Backend-owned classification color mappings and complete plan generation.

use std::sync::Arc;

use viewport_protocol::{
    ClassificationColorEntry, ClassificationColorIntent, ClassificationColorSource, ColorRgb8,
    MAX_CLASSIFICATION_COLOR_ENTRIES,
};

use super::BimQueryError;
use super::classification::ClassificationColorGroup;

const DEFAULT_PROFILE_ID: &str = "default";
const PROFILE_FALLBACK: ColorRgb8 = ColorRgb8::new(0x94, 0xA3, 0xB8);
const AUTO_PALETTE: [ColorRgb8; 8] = [
    ColorRgb8::new(0x22, 0xD3, 0xEE),
    ColorRgb8::new(0xC0, 0x84, 0xFC),
    ColorRgb8::new(0xFB, 0x71, 0xA7),
    ColorRgb8::new(0xFD, 0xBA, 0x74),
    ColorRgb8::new(0x6E, 0xE7, 0xB7),
    ColorRgb8::new(0xFE, 0xF0, 0x8A),
    ColorRgb8::new(0xFE, 0x8A, 0xA8),
    ColorRgb8::new(0xA5, 0xF3, 0xFC),
];

struct ProfileMapping {
    level: &'static str,
    value: &'static str,
    color: ColorRgb8,
}

const DEFAULT_PROFILE: &[ProfileMapping] = &[
    ProfileMapping {
        level: "category",
        value: "Walls",
        color: ColorRgb8::new(0x38, 0xBD, 0xF8),
    },
    ProfileMapping {
        level: "category",
        value: "Doors",
        color: ColorRgb8::new(0xA7, 0x8B, 0xFA),
    },
    ProfileMapping {
        level: "category",
        value: "Windows",
        color: ColorRgb8::new(0xF4, 0x72, 0xB6),
    },
    ProfileMapping {
        level: "category",
        value: "Floors",
        color: ColorRgb8::new(0xFB, 0xA7, 0x4A),
    },
    ProfileMapping {
        level: "family",
        value: "Basic Wall",
        color: ColorRgb8::new(0x38, 0xBD, 0xF8),
    },
    ProfileMapping {
        level: "family",
        value: "Single-Flush",
        color: ColorRgb8::new(0xA7, 0x8B, 0xFA),
    },
    ProfileMapping {
        level: "type",
        value: "Basic Wall",
        color: ColorRgb8::new(0x34, 0xD3, 0x99),
    },
    ProfileMapping {
        level: "type",
        value: "Door",
        color: ColorRgb8::new(0xF9, 0xFA, 0xB2),
    },
];

pub(super) fn entries(
    groups: &Arc<Vec<ClassificationColorGroup>>,
    intent: &ClassificationColorIntent,
) -> Result<Vec<ClassificationColorEntry>, BimQueryError> {
    let Some(active_level) = intent.active_level.as_deref() else {
        return Err(BimQueryError::Invalid(
            viewport_protocol::ProtocolValidationError::EmptyField {
                field: "classification_color.active_level",
            },
        ));
    };
    if matches!(intent.source, ClassificationColorSource::None) {
        return Ok(Vec::new());
    }
    let mut output = Vec::with_capacity(groups.len());
    for group in groups.iter() {
        let Some((_, value)) = group.levels.iter().find(|(id, _)| id == active_level) else {
            continue;
        };
        let color = match &intent.source {
            ClassificationColorSource::None => unreachable!(),
            ClassificationColorSource::Profile(profile) => {
                profile_color(profile, active_level, value)?
            }
            ClassificationColorSource::Auto => auto_color(intent.generation, active_level, value),
        };
        output.push(ClassificationColorEntry {
            anchor: group.anchor.clone(),
            color,
        });
    }
    if output.len() > MAX_CLASSIFICATION_COLOR_ENTRIES {
        return Err(BimQueryError::TooManyResults {
            kind: "classification color entries",
            limit: MAX_CLASSIFICATION_COLOR_ENTRIES,
        });
    }
    Ok(output)
}

fn profile_color(profile: &str, level: &str, value: &str) -> Result<ColorRgb8, BimQueryError> {
    if profile != DEFAULT_PROFILE_ID {
        return Err(BimQueryError::Invalid(
            viewport_protocol::ProtocolValidationError::InvalidInput {
                field: "classification_color.profile",
            },
        ));
    }
    Ok(DEFAULT_PROFILE
        .iter()
        .find(|mapping| mapping.level == level && mapping.value == value)
        .map_or(PROFILE_FALLBACK, |mapping| mapping.color))
}

fn auto_color(generation: u64, level: &str, value: &str) -> ColorRgb8 {
    let mut hash = 0xcbf29ce484222325_u64;
    for bytes in [b"auto".as_slice(), level.as_bytes(), value.as_bytes()] {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= generation;
    hash = hash.wrapping_mul(0x100000001b3);
    AUTO_PALETTE[(hash as usize) % AUTO_PALETTE.len()]
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/bim/classification_color_inline_tests.rs"]
mod tests;
