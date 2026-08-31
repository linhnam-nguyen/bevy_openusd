//! Semantic hierarchy metadata projected from the authoritative USD Stage.

use bevy::prelude::{Entity, World};
use openusd::sdf::Value;
use openusd::usd::PrimPredicate;
use std::collections::HashMap;

use super::super::RouteCtx;
use super::UsdDisplayName;
use crate::prim_ref::{
    USDHUB_HIERARCHY_ROLE_METADATA, USDHUB_TRANSPARENT_SOURCE_ROLE, UsdHierarchyTarget,
    UsdTransparentHierarchyNode,
};

const DISPLAY_NAME_FIELD: &str = "ui:displayName";
const TARGET_KIND_FIELD: &str = "usdhub:targetKind";
const TARGET_ID_FIELD: &str = "usdhub:targetId";

/// Stage-owned metadata projected into Bevy is indexed once per composed Stage.
///
/// The composed attribute query remains authoritative for ordinary authored
/// attributes. The index covers the legacy prim-spec metadata representation,
/// whose value is otherwise only available by scanning every layer in the
/// stack. Each prim-stack site is recorded under the composed Stage path, so
/// referenced descendants keep their display names after namespace mapping.
/// Keeping that work at cache-build time makes the per-prim projection read
/// O(1), independent of the number of layers.
#[derive(bevy::ecs::resource::Resource, Debug, Default, Clone)]
pub(crate) struct HierarchyMetadataIndex {
    stage_address: usize,
    revision: u64,
    display_names: HashMap<String, String>,
}

/// Revision of the live Stage represented by the cached metadata index.
///
/// The live reconciliation boundary updates this resource once per drained
/// [`crate::live::StageChangeBatch`]. A same-address Stage therefore still invalidates its
/// metadata cache after in-place authoring, while every prim in that batch
/// shares one rebuilt index.
#[derive(bevy::ecs::resource::Resource, Debug, Default, Clone, Copy, Eq, PartialEq)]
pub(crate) struct HierarchyMetadataRevision(pub(crate) u64);

impl HierarchyMetadataIndex {
    #[cfg(test)]
    pub(super) fn display_name(&self, path: &str) -> Option<&str> {
        self.display_names.get(path).map(String::as_str)
    }
}

pub(super) fn prepare_metadata_index(stage: &openusd::usd::Stage, world: &mut World) {
    let stage_address = std::ptr::from_ref(stage) as usize;
    let revision = world
        .get_resource::<HierarchyMetadataRevision>()
        .map_or(0, |revision| revision.0);
    let needs_rebuild = world
        .get_resource::<HierarchyMetadataIndex>()
        .is_none_or(|index| index.stage_address != stage_address || index.revision != revision);
    if needs_rebuild {
        world.insert_resource(build_metadata_index(stage, stage_address, revision));
    }
}

pub(super) fn note_metadata_revision(world: &mut World, revision: u64) {
    world.insert_resource(HierarchyMetadataRevision(revision));
}

fn build_metadata_index(
    stage: &openusd::usd::Stage,
    stage_address: usize,
    revision: u64,
) -> HierarchyMetadataIndex {
    let mut display_names = HashMap::new();
    let _ = stage.traverse(PrimPredicate::ALL, |composed_path| {
        let prim = stage.prim(composed_path.clone());
        let Ok(prim_stack) = prim.prim_stack() else {
            return;
        };
        for (identifier, spec_path) in prim_stack {
            let Some(layer) = stage.layer(&identifier) else {
                continue;
            };
            let data = layer.data();
            let Ok(Some(value)) = data.try_field(&spec_path, DISPLAY_NAME_FIELD) else {
                continue;
            };
            if let Some(value) = as_text(value.into_owned()) {
                // Prim::prim_stack is strength ordered, so the first opinion
                // wins while the composed path preserves reference mapping.
                display_names.insert(composed_path.as_str().to_owned(), value);
                break;
            }
        }
    });
    HierarchyMetadataIndex {
        stage_address,
        revision,
        display_names,
    }
}

/// Project semantic labels and explicit USDHub implementation roles alongside
/// the normal visibility route. These components are disposable projections;
/// the composed Stage remains authoritative for both values.
pub(super) fn apply_metadata(ctx: &RouteCtx, world: &mut World, entity: Entity) {
    prepare_metadata_index(ctx.stage, world);
    let display_name = read_display_name(ctx, world.get_resource::<HierarchyMetadataIndex>());
    let transparent_source = ctx
        .stage
        .prim(ctx.path.clone())
        .custom_data()
        .ok()
        .flatten()
        .and_then(|value| match value {
            Value::Dictionary(data) => data
                .get(USDHUB_HIERARCHY_ROLE_METADATA)
                .and_then(Value::as_str)
                .map(|value| value == USDHUB_TRANSPARENT_SOURCE_ROLE),
            _ => None,
        })
        .unwrap_or(false);
    let target = ctx
        .stage
        .prim(ctx.path.clone())
        .custom_data()
        .ok()
        .flatten()
        .and_then(|value| match value {
            Value::Dictionary(data) => Some(UsdHierarchyTarget::new(
                data.get(TARGET_KIND_FIELD)?.as_str()?,
                data.get(TARGET_ID_FIELD)?.as_str()?,
            )),
            _ => None,
        })
        .filter(|target| !target.kind.is_empty() && !target.id.is_empty());

    let Ok(mut entity) = world.get_entity_mut(entity) else {
        return;
    };
    if let Some(display_name) = display_name {
        entity.insert(UsdDisplayName(display_name));
    } else {
        entity.remove::<UsdDisplayName>();
    }
    if transparent_source {
        entity.insert(UsdTransparentHierarchyNode);
    } else {
        entity.remove::<UsdTransparentHierarchyNode>();
    }
    if let Some(target) = target {
        entity.insert(target);
    } else {
        entity.remove::<UsdHierarchyTarget>();
    }
}

/// Read both representations found in USDHub's existing files: ordinary USD
/// `ui:displayName` attributes and the prim metadata field authored by the
/// storage-v2 project layer. The index is prepared at the Stage→Bevy
/// projection boundary, so this per-prim read never scans the layer stack.
fn as_text(value: Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value),
        Value::Token(value) => Some(value.as_str().to_owned()),
        _ => None,
    }
}

fn read_display_name(ctx: &RouteCtx, index: Option<&HierarchyMetadataIndex>) -> Option<String> {
    ctx.stage
        .prim(ctx.path.clone())
        .attribute(DISPLAY_NAME_FIELD)
        .get::<Value>()
        .ok()
        .flatten()
        .and_then(as_text)
        .or_else(|| index?.display_names.get(ctx.prim_str()).cloned())
}
