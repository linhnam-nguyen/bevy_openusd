//! Semantic hierarchy metadata projected from the authoritative USD Stage.

use bevy::prelude::{Entity, World};
use openusd::vt::Value;
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

#[cfg(test)]
#[path = "../../tests/unit/route/hierarchy_support.rs"]
mod test_support;

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
        if let Ok(Some(value)) = read_legacy_display_name(stage, &composed_path) {
            display_names.insert(composed_path.as_str().to_owned(), value);
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
        .ok()
        .and_then(|prim| prim.metadata::<Value>("customData").ok().flatten())
        .and_then(|value| match value {
            Value::Dictionary(data) => data
                .get(USDHUB_HIERARCHY_ROLE_METADATA)
                .and_then(Value::as_str)
                .map(|value| value == USDHUB_TRANSPARENT_SOURCE_ROLE),
            _ => None,
        })
        .unwrap_or(false);
    let target = read_target(ctx).ok().flatten();

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
    crate::read::util::decode_metadata_string(&value)
}

fn read_display_name(ctx: &RouteCtx, index: Option<&HierarchyMetadataIndex>) -> Option<String> {
    read_attribute_display_name(ctx)
        .ok()
        .flatten()
        .or_else(|| index?.display_names.get(ctx.prim_str()).cloned())
}

fn read_attribute_display_name(ctx: &RouteCtx) -> anyhow::Result<Option<String>> {
    Ok(ctx
        .stage
        .prim(ctx.path.clone())?
        .attribute(DISPLAY_NAME_FIELD)
        .get::<Value>()?
        .and_then(as_text))
}

fn read_legacy_display_name(
    stage: &openusd::usd::Stage,
    composed_path: &openusd::sdf::Path,
) -> anyhow::Result<Option<String>> {
    let prim = stage.prim(composed_path.clone())?;
    // Prim::prim_stack is strength ordered; keep the first decodable opinion
    // at the composed path, including references. A failed read is not absence.
    for site in prim.prim_stack()? {
        let Some(spec) = site.layer.prim(site.path.clone())? else {
            continue;
        };
        let Some(value) = spec.field(DISPLAY_NAME_FIELD)? else {
            continue;
        };
        if let Some(value) = as_text(value) {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn read_target(ctx: &RouteCtx) -> anyhow::Result<Option<UsdHierarchyTarget>> {
    Ok(ctx
        .stage
        .prim(ctx.path.clone())?
        .metadata::<Value>("customData")?
        .and_then(|value| match value {
            Value::Dictionary(data) => Some(UsdHierarchyTarget::new(
                data.get(TARGET_KIND_FIELD)?.as_str()?,
                data.get(TARGET_ID_FIELD)?.as_str()?,
            )),
            _ => None,
        })
        .filter(|target| !target.kind.is_empty() && !target.id.is_empty()))
}

pub(super) fn read_authored_hierarchy_metadata(
    stage: &openusd::usd::Stage,
    prim_path: &str,
) -> anyhow::Result<super::AuthoredHierarchyMetadata> {
    let path = openusd::sdf::path(prim_path)?;
    anyhow::ensure!(
        stage.prim(path.clone())?.is_defined()?,
        "cached hierarchy prim {prim_path} is not defined in the composed Stage"
    );
    let ctx = RouteCtx::new(stage, &path);
    let display_name = match read_attribute_display_name(&ctx)? {
        Some(name) => Some(name),
        None => read_legacy_display_name(stage, &path)?,
    };
    Ok(super::AuthoredHierarchyMetadata {
        display_name,
        target: read_target(&ctx)?,
    })
}
