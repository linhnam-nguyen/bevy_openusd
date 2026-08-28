//! Stage-backed BIM property authoring locators.
//!
//! A semantic property name is a stable key supplied by the extractor. It is
//! not a display label and it is not converted through the Bevy scene index.
//! This module resolves that key directly against the current OpenUSD stage
//! before any edit is admitted.

use std::fmt;

use openusd::usd::Stage;
use viewport_protocol::SceneAnchor;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BimEditability {
    Editable,
    NonEditable { reason: BimNonEditableReason },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BimNonEditableReason {
    DerivedProperty,
    NonCustomAttribute,
    UnsupportedType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BimAuthoringLocator {
    pub(super) target: SceneAnchor,
    pub(super) property_key: String,
    pub(super) prim_path: String,
    pub(super) attribute_path: String,
    pub(super) type_name: Option<String>,
    pub(super) editability: BimEditability,
}

impl BimAuthoringLocator {
    pub(super) fn is_editable(&self) -> bool {
        self.editability == BimEditability::Editable
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum BimAuthoringError {
    InvalidPropertyKey,
    InvalidPrimPath(String),
    PrimNotFound(String),
    AttributeNotFound {
        prim_path: String,
        property_key: String,
    },
    Stage(String),
}

impl fmt::Display for BimAuthoringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPropertyKey => formatter.write_str("BIM property key is invalid"),
            Self::InvalidPrimPath(path) => write!(formatter, "invalid BIM prim path: {path}"),
            Self::PrimNotFound(path) => write!(formatter, "BIM prim not found: {path}"),
            Self::AttributeNotFound {
                prim_path,
                property_key,
            } => write!(
                formatter,
                "BIM property {property_key} is missing on prim {prim_path}"
            ),
            Self::Stage(error) => write!(formatter, "BIM stage inspection failed: {error}"),
        }
    }
}

impl std::error::Error for BimAuthoringError {}

pub(super) fn resolve_bim_authoring_locator(
    stage: &Stage,
    target: &SceneAnchor,
    property_key: &str,
) -> Result<BimAuthoringLocator, BimAuthoringError> {
    target
        .validate()
        .map_err(|_| BimAuthoringError::InvalidPrimPath(target.prim_path.clone()))?;
    if property_key.trim().is_empty() || property_key.contains('\0') {
        return Err(BimAuthoringError::InvalidPropertyKey);
    }

    let prim_path = openusd::sdf::path(&target.prim_path)
        .map_err(|_| BimAuthoringError::InvalidPrimPath(target.prim_path.clone()))?;
    let prim = stage.prim(prim_path);
    if !prim.is_valid().map_err(stage_error)? {
        return Err(BimAuthoringError::PrimNotFound(target.prim_path.clone()));
    }

    let attribute_path = format!("{}.{}", target.prim_path, property_key);
    if is_derived_property(property_key) {
        return Ok(BimAuthoringLocator {
            target: target.clone(),
            property_key: property_key.to_owned(),
            prim_path: target.prim_path.clone(),
            attribute_path,
            type_name: None,
            editability: BimEditability::NonEditable {
                reason: BimNonEditableReason::DerivedProperty,
            },
        });
    }

    let has_attribute = prim
        .property_names()
        .map_err(stage_error)?
        .iter()
        .any(|name| name.as_str() == property_key);
    if !has_attribute {
        return Err(BimAuthoringError::AttributeNotFound {
            prim_path: target.prim_path.clone(),
            property_key: property_key.to_owned(),
        });
    }

    let attribute = prim.attribute(property_key);
    let type_name = attribute
        .type_name()
        .map_err(stage_error)?
        .map(|name| name.as_str().to_owned());
    let editability = if !attribute.is_custom().map_err(stage_error)? {
        BimEditability::NonEditable {
            reason: BimNonEditableReason::NonCustomAttribute,
        }
    } else if !type_name
        .as_deref()
        .is_some_and(is_supported_attribute_type)
    {
        BimEditability::NonEditable {
            reason: BimNonEditableReason::UnsupportedType,
        }
    } else {
        BimEditability::Editable
    };

    Ok(BimAuthoringLocator {
        target: target.clone(),
        property_key: property_key.to_owned(),
        prim_path: target.prim_path.clone(),
        attribute_path,
        type_name,
        editability,
    })
}

fn is_derived_property(property_key: &str) -> bool {
    matches!(
        property_key,
        "semantic.category"
            | "semantic.family"
            | "semantic.type_name"
            | "semantic.type_id"
            | "semantic.display_name"
            | "bim.element_id"
            | "bim.family_name"
    )
}

/// The supported set is intentionally the same scalar/vector/array surface
/// accepted by the viewport editor value converter. A resolver must not mark
/// a property editable when the normal authoring path cannot encode it.
fn is_supported_attribute_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "bool"
            | "uchar"
            | "int"
            | "uint"
            | "int64"
            | "uint64"
            | "float"
            | "double"
            | "string"
            | "token"
            | "asset"
            | "timecode"
            | "float2"
            | "float3"
            | "point3f"
            | "vector3f"
            | "normal3f"
            | "color3f"
            | "float4"
            | "color4f"
            | "double2"
            | "double3"
            | "point3d"
            | "vector3d"
            | "normal3d"
            | "color3d"
            | "double4"
            | "color4d"
            | "int2"
            | "int3"
            | "int4"
            | "quatf"
            | "quatd"
            | "matrix2d"
            | "matrix3d"
            | "matrix4d"
            | "path"
            | "bool[]"
            | "int[]"
            | "uint[]"
            | "int64[]"
            | "uint64[]"
            | "float[]"
            | "double[]"
            | "string[]"
            | "token[]"
            | "asset[]"
            | "float3[]"
            | "double3[]"
            | "matrix4d[]"
    )
}

fn stage_error(error: impl fmt::Display) -> BimAuthoringError {
    BimAuthoringError::Stage(error.to_string())
}

#[cfg(test)]
mod tests {
    use openusd::sdf::Value;
    use viewport_protocol::SceneAnchor;

    use super::*;

    fn stage_with_attribute(type_name: &str, custom: bool) -> Stage {
        let stage = Stage::builder()
            .in_memory("bim_authoring_locator.usda")
            .expect("stage opens");
        stage
            .define_prim("/World/Wall")
            .expect("prim defines")
            .set_type_name("Xform")
            .expect("prim type authors");
        stage
            .prim(openusd::sdf::path("/World/Wall").expect("path parses"))
            .create_attribute("Width", type_name)
            .expect("attribute creates")
            .set_custom(custom)
            .expect("custom flag authors")
            .set(Value::Double(1.0))
            .expect("value authors");
        stage
    }

    fn target() -> SceneAnchor {
        SceneAnchor::active_session("/World/Wall")
    }

    #[test]
    fn resolves_editable_custom_attribute_by_stable_key() {
        let stage = stage_with_attribute("double", true);
        let locator = resolve_bim_authoring_locator(&stage, &target(), "Width")
            .expect("custom scalar resolves");

        assert_eq!(locator.property_key, "Width");
        assert_eq!(locator.attribute_path, "/World/Wall.Width");
        assert_eq!(locator.type_name.as_deref(), Some("double"));
        assert!(locator.is_editable());
    }

    #[test]
    fn derived_semantic_property_is_explicitly_non_editable() {
        let stage = stage_with_attribute("double", true);
        let locator = resolve_bim_authoring_locator(&stage, &target(), "semantic.category")
            .expect("derived field resolves as a capability descriptor");

        assert_eq!(
            locator.editability,
            BimEditability::NonEditable {
                reason: BimNonEditableReason::DerivedProperty
            }
        );
        assert!(!locator.is_editable());
    }

    #[test]
    fn missing_attribute_is_rejected_without_guessing_a_target() {
        let stage = stage_with_attribute("double", true);
        assert!(matches!(
            resolve_bim_authoring_locator(&stage, &target(), "Missing"),
            Err(BimAuthoringError::AttributeNotFound { .. })
        ));
    }

    #[test]
    fn non_custom_and_unsupported_attributes_are_not_editable() {
        let non_custom = stage_with_attribute("double", false);
        let locator = resolve_bim_authoring_locator(&non_custom, &target(), "Width")
            .expect("schema attribute resolves");
        assert_eq!(
            locator.editability,
            BimEditability::NonEditable {
                reason: BimNonEditableReason::NonCustomAttribute
            }
        );

        let unsupported = stage_with_attribute("dictionary", true);
        let locator = resolve_bim_authoring_locator(&unsupported, &target(), "Width")
            .expect("unsupported attribute resolves");
        assert_eq!(
            locator.editability,
            BimEditability::NonEditable {
                reason: BimNonEditableReason::UnsupportedType
            }
        );
    }
}
