use std::fmt;

use openusd::usd::Stage;
use usd_model::CanonicalValue;
use viewport_protocol::SceneAnchor;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BimEditability {
    Editable,
    NonEditable { reason: BimNonEditableReason },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BimNonEditableReason {
    DerivedProperty,
    NonCustomAttribute,
    UnsupportedType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BimAuthoringLocator {
    pub(crate) target: SceneAnchor,
    pub(crate) property_key: String,
    pub(crate) prim_path: String,
    pub(crate) attribute_path: String,
    pub(crate) type_name: Option<String>,
    pub(crate) editability: BimEditability,
}

impl BimAuthoringLocator {
    pub(crate) fn is_editable(&self) -> bool {
        self.editability == BimEditability::Editable
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum BimAuthoringError {
    InvalidPropertyKey,
    InvalidPrimPath(String),
    PrimNotFound(String),
    AttributeNotFound {
        prim_path: String,
        property_key: String,
    },
    AttributeValueMissing {
        attribute_path: String,
    },
    ExpectedValueMismatch {
        property_key: String,
        expected: CanonicalValue,
        current: CanonicalValue,
    },
    NonEditable {
        property_key: String,
        reason: BimNonEditableReason,
    },
    InvalidUnit(String),
    InvalidValue(String),
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
            Self::AttributeValueMissing { attribute_path } => write!(
                formatter,
                "BIM property has no authored value: {attribute_path}"
            ),
            Self::ExpectedValueMismatch {
                property_key,
                expected,
                current,
            } => write!(
                formatter,
                "BIM property {property_key} is stale: expected {expected:?}, current {current:?}"
            ),
            Self::NonEditable {
                property_key,
                reason,
            } => write!(
                formatter,
                "BIM property {property_key} is not editable ({reason:?})"
            ),
            Self::InvalidUnit(error) => write!(formatter, "invalid BIM edit unit: {error}"),
            Self::InvalidValue(error) => write!(formatter, "invalid BIM edit value: {error}"),
            Self::Stage(error) => write!(formatter, "BIM stage inspection failed: {error}"),
        }
    }
}

pub(crate) fn resolve_bim_authoring_locator(
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

pub(super) fn stage_error(error: impl fmt::Display) -> BimAuthoringError {
    BimAuthoringError::Stage(error.to_string())
}

impl std::error::Error for BimAuthoringError {}
