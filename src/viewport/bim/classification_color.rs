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
mod tests {
    use super::*;
    use viewport_protocol::{BimFieldKey, ClassificationLevel, ClassificationRecipe};

    use crate::viewport::bim::BimReadService;
    use crate::viewport::bim::test_fixtures::snapshot;

    #[test]
    fn profile_uses_defined_level_and_value_mapping_for_unpaged_entities() {
        let snapshot = snapshot();
        let recipe = ClassificationRecipe::new(vec![
            ClassificationLevel::new("category", BimFieldKey::Category),
            ClassificationLevel::new("type", BimFieldKey::Type),
        ]);
        let mut service = BimReadService::new(&snapshot);
        let entries = service
            .classification_color_entries(
                &recipe,
                &ClassificationColorIntent {
                    source: ClassificationColorSource::Profile("default".to_owned()),
                    active_level: Some("category".to_owned()),
                    generation: 0,
                },
            )
            .expect("profile plan");
        assert_eq!(entries.len(), snapshot.entities.len());
        assert!(entries.iter().any(|entry| {
            entry.anchor.prim_path.contains("Wall")
                && entry.color == ColorRgb8::new(0x38, 0xBD, 0xF8)
        }));
    }

    #[test]
    fn active_level_changes_group_color_identity() {
        let snapshot = snapshot();
        let recipe = ClassificationRecipe::new(vec![
            ClassificationLevel::new("category", BimFieldKey::Category),
            ClassificationLevel::new("type", BimFieldKey::Type),
        ]);
        let mut category_service = BimReadService::new(&snapshot);
        let category = category_service
            .classification_color_entries(
                &recipe,
                &ClassificationColorIntent {
                    source: ClassificationColorSource::Auto,
                    active_level: Some("category".to_owned()),
                    generation: 3,
                },
            )
            .expect("category plan");
        let mut type_service = BimReadService::new(&snapshot);
        let types = type_service
            .classification_color_entries(
                &recipe,
                &ClassificationColorIntent {
                    source: ClassificationColorSource::Auto,
                    active_level: Some("type".to_owned()),
                    generation: 3,
                },
            )
            .expect("type plan");
        assert_ne!(category, types);
    }

    #[test]
    fn auto_changes_only_when_the_explicit_generation_changes() {
        let snapshot = snapshot();
        let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
            "category",
            BimFieldKey::Category,
        )]);
        let mut first_service = BimReadService::new(&snapshot);
        let first = first_service
            .classification_color_entries(
                &recipe,
                &ClassificationColorIntent {
                    source: ClassificationColorSource::Auto,
                    active_level: Some("category".to_owned()),
                    generation: 7,
                },
            )
            .expect("first auto plan");
        let mut same_service = BimReadService::new(&snapshot);
        let same = same_service
            .classification_color_entries(
                &recipe,
                &ClassificationColorIntent {
                    source: ClassificationColorSource::Auto,
                    active_level: Some("category".to_owned()),
                    generation: 7,
                },
            )
            .expect("same auto plan");
        let mut refreshed_service = BimReadService::new(&snapshot);
        let refreshed = refreshed_service
            .classification_color_entries(
                &recipe,
                &ClassificationColorIntent {
                    source: ClassificationColorSource::Auto,
                    active_level: Some("category".to_owned()),
                    generation: 8,
                },
            )
            .expect("refreshed auto plan");
        assert_eq!(first, same);
        assert_ne!(first, refreshed);
    }
}
