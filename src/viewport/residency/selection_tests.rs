use super::*;
use crate::project::{
    blob_store::prepare_mesh_payload,
    cache::{ProjectCacheIdentity, SceneCacheStore},
    cache_contract::{
        CachedTransform, ProjectCacheTarget, SCENE_CACHE_INDEX_SCHEMA_VERSION,
        SCENE_SPATIAL_INDEX_SCHEMA_VERSION, SceneCacheAddress, SceneCacheBlobRef,
        SceneCacheDescriptorV3, SceneCacheEntry, SceneCacheEntryKind, SceneCacheIndex,
        SceneCacheOccurrence, SceneCacheState, SceneSpatialIndex,
    },
    catalog::manifest_store::ManifestStore,
};
use crate::viewport::residency::{
    PayloadResidencyState, ResidencyAuthority,
    repair::{
        TargetedRepairQueue, drain_cached_residency_completions,
        drain_targeted_repair_persistence_completions, process_targeted_residency_repairs,
    },
    repair_persistence::TargetedRepairPersistenceWorker,
    worker::CachedResidencyWorker,
};
use bevy::{asset::Assets, mesh::Mesh, prelude::*};
use std::{collections::HashMap, path::Path, thread, time::Duration};
use tempfile::tempdir;
use usd_bevy::UsdSnippet;
use usd_model::{BlobId, Bounds3, HashDigest};
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, SceneMemberId,
    ScenePlacementTransform, StorageKey,
};

fn digest(value: usize) -> HashDigest {
    let mut bytes = [0; HashDigest::BYTE_LEN];
    bytes[..8].copy_from_slice(&(value as u64).to_le_bytes());
    HashDigest::new(bytes)
}

fn owned_entry(
    scene: SceneId,
    path: &str,
    hash: HashDigest,
    bytes: Option<u64>,
) -> SceneCacheEntry {
    let address = SceneCacheAddress {
        scene_id: scene,
        occurrence: SceneCacheOccurrence::PrimPath(path.to_owned()),
    };
    let geometry = bytes.map(|byte_size| SceneCacheBlobRef {
        blob_id: BlobId(hash.to_hex()),
        byte_size,
    });
    SceneCacheEntry {
        address,
        parent: None,
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: Some(Bounds3 {
            min: [-1.0; 3],
            max: [1.0; 3],
        }),
        cacheable: geometry.is_some(),
        bim_enabled: false,
        geometry,
        material: None,
        animation: None,
        semantic_key: None,
        kind: SceneCacheEntryKind::OwnedPrim {
            prim_path: path.to_owned(),
        },
        content_hash: Some(hash),
    }
}
fn entry(scene: SceneId, path: &str, value: usize) -> SceneCacheEntry {
    owned_entry(scene, path, digest(value), Some(8))
}
fn presentation(
    scene_id: SceneId,
    generation: u64,
    entries: Vec<SceneCacheEntry>,
) -> SceneCachePresentation {
    SceneCachePresentation {
        scene_id,
        generation,
        state: SceneCacheState::Ready,
        entries,
    }
}
fn context(root: &Path) -> ActiveProjectCacheContext {
    ActiveProjectCacheContext::from_identity(
        root.to_path_buf(),
        ProjectCacheIdentity {
            target: ProjectCacheTarget::ProjectRoot,
            target_content_hash: digest(91),
            profile: viewport_protocol::RuntimeProfile::NativeMedium,
            config_hash: digest(92),
        },
    )
}
fn selection_app(
    selection: SelectedTargets,
    index: SceneAnchorIndex,
    scene: SceneCachePresentation,
) -> App {
    let mut app = App::new();
    app.insert_resource(selection)
        .insert_resource(index)
        .insert_resource(scene)
        .insert_resource(ResidencyAuthority::default())
        .init_resource::<SelectionResidencyState>()
        .add_systems(Update, sync_selected_residency);
    let (scene_id, generation) = {
        let presentation = app.world().resource::<SceneCachePresentation>();
        (presentation.scene_id, presentation.generation)
    };
    app.world_mut()
        .resource_mut::<ResidencyAuthority>()
        .install_scene(scene_id, generation, Vec::new());
    app
}
fn child_entry(parent: SceneId, child: SceneId, member: SceneMemberId) -> SceneCacheEntry {
    let address = SceneCacheAddress {
        scene_id: parent,
        occurrence: SceneCacheOccurrence::Member(member),
    };
    let kind = SceneCacheEntryKind::ChildScene {
        scene_id: child,
        member_id: member,
    };
    SceneCacheEntry {
        address,
        parent: None,
        transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
        bounds: None,
        cacheable: false,
        bim_enabled: false,
        geometry: None,
        material: None,
        animation: None,
        semantic_key: None,
        kind,
        content_hash: None,
    }
}
fn publish_index(root: &Path, index: SceneCacheIndex) {
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id: index.scene_id,
        generation: index.generation,
        entries: Vec::new(),
    };
    let mut descriptor =
        SceneCacheDescriptorV3::invalidated(index.scene_id, index.generation, digest(90));
    descriptor.state = SceneCacheState::Partial;
    descriptor.prim_count = index.entries.len() as u64;
    descriptor.cacheable_count =
        index.entries.iter().filter(|entry| entry.cacheable).count() as u64;
    SceneCacheStore::new(root)
        .publish_generation(&descriptor, &index, &spatial)
        .expect("publish test Scene cache index");
}
fn write_manifest(root: &Path, parent: SceneId, child: SceneId) -> anyhow::Result<()> {
    let scenes = [(parent, "parent", "Parent"), (child, "child", "Child")]
        .into_iter()
        .map(|(id, key, name)| {
            Ok(SceneManifestEntry {
                id,
                storage_key: StorageKey::new(key)?,
                display_name: name.into(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Selection",
        ProjectRoot::Scene(parent),
        scenes,
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(root, &manifest)
}
fn child_live_stage() -> anyhow::Result<usd_bevy::LiveStage> {
    let source = r#"#usda 1.0
def Xform "SceneRoot" {
    def Mesh "Leaf" {
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
    }
}
"#;
    Ok(usd_bevy::LiveStage::new(
        UsdSnippet::new(source).open_stage()?,
    ))
}
fn mesh_hash(mesh: &Mesh) -> HashDigest {
    HashDigest::from_hex(&prepare_mesh_payload(mesh).expect("mesh prepares").blob_id.0)
        .expect("mesh hash decodes")
}
fn production_selection_app(
    selection: SelectedTargets,
    index: SceneAnchorIndex,
    scene: SceneCachePresentation,
    root: &Path,
    live: usd_bevy::LiveStage,
) -> App {
    let mut app = selection_app(selection, index, scene);
    app.insert_resource(context(root))
        .insert_resource(CachedResidencyWorker::new())
        .insert_resource(TargetedRepairPersistenceWorker::new())
        .insert_resource(TargetedRepairQueue::default());
    app.world_mut().insert_non_send(live);
    app.add_systems(
        Update,
        (
            super::super::dispatch_cached_residency_loads,
            drain_cached_residency_completions,
            process_targeted_residency_repairs,
            drain_targeted_repair_persistence_completions,
        )
            .chain()
            .after(sync_selected_residency),
    );
    app
}

fn single_target_fixture(
    scene: SceneId,
    path: &str,
    value: usize,
) -> (
    SelectedTargets,
    SceneAnchor,
    SceneAnchorIndex,
    SelectionResidencyState,
) {
    let target = SceneAnchor::active_session(path);
    let mut selection = SelectedTargets::default();
    selection
        .add(target.clone(), true)
        .expect("selection is valid");
    let index = SceneAnchorIndex::from_test_entity(target.clone(), Entity::from_bits(1));
    let presentation = presentation(scene, 1, vec![entry(scene, path, value)]);
    let state = SelectionResidencyState {
        generation: Some(1),
        selection_revision: Some(selection.revision()),
        scene_revision: Some(index.revision()),
        lookup: build_lookup(&presentation, None),
        ..Default::default()
    };
    (selection, target, index, state)
}

#[test]
#[rustfmt::skip]
fn production_selection_follows_distinct_child_members_through_c7_repair() -> anyhow::Result<()> {
    let d = tempdir()?; let parent = SceneId::new_v4(); let child = SceneId::new_v4(); let first_member = SceneMemberId::new_v4(); let second_member = SceneMemberId::new_v4();
    write_manifest(d.path(), parent, child)?; let live = child_live_stage()?; let payload = usd_bevy::extract_render_payloads_for_paths(&live.stage, &["/SceneRoot/Leaf"])?.into_iter().next().expect("child payload"); let child_hash = mesh_hash(&payload.mesh);
    publish_index(d.path(), SceneCacheIndex { schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION, scene_id: child, generation: 7, entries: vec![owned_entry(child, "/SceneRoot/Leaf", child_hash, None)] });
    let member_path = |member| format!("{}/SceneRoot/Leaf", crate::project::scene::authoring::scene_member_path(member)); let mut parent_entries = vec![child_entry(parent, child, first_member), child_entry(parent, child, second_member)]; parent_entries.sort_by(|left, right| left.address.cmp(&right.address));
    publish_index(d.path(), SceneCacheIndex { schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION, scene_id: parent, generation: 3, entries: parent_entries.clone() });
    let first = SceneAnchor::active_session(member_path(first_member)); let second = SceneAnchor::active_session(member_path(second_member)); let missing = SceneAnchor::active_session(format!("{}/SceneRoot/Missing", crate::project::scene::authoring::scene_member_path(first_member)));
    let mut selected = SelectedTargets::default(); selected.add_many(vec![first.clone(), second.clone(), missing.clone()], Some(first.clone()))?; let index = SceneAnchorIndex::from_test_entities(vec![(first.clone(), Entity::from_bits(1)), (second.clone(), Entity::from_bits(2)), (missing.clone(), Entity::from_bits(3))]);
    let mut app = production_selection_app(selected, index, presentation(parent, 3, parent_entries), d.path(), live); app.update();
    let child_key = { let state = app.world().resource::<SelectionResidencyState>(); let first_key = *state.selected.get(&first).expect("first child payload"); assert_eq!(first_key.scene_id, child); assert_eq!(state.selected.get(&second), Some(&first_key)); assert_eq!(state.selected.len(), 2); assert!(!state.selected.contains_key(&missing)); assert!(state.selected.values().all(|key| key.scene_id == child)); assert_eq!(first_key.blob_hash, child_hash); first_key };
    let parent_key = ScenePayloadKey { scene_id: parent, blob_hash: child_hash }; assert_eq!(app.world().resource::<ResidencyAuthority>().state(&parent_key), None);
    let child_job = super::super::loader::LoadJob { key: child_key, generation: 7, cpu_bytes: 0, gpu_bytes: 0 }; let parent_job = super::super::loader::LoadJob { key: parent_key, generation: 3, cpu_bytes: 0, gpu_bytes: 0 };
    for _ in 0..512 { app.update(); if app.world().resource::<ResidencyAuthority>().state(&child_key) == Some(PayloadResidencyState::RepairWaiting) { break; } thread::sleep(Duration::from_millis(1)); }
    let authority = app.world().resource::<ResidencyAuthority>(); assert!(authority.repair_waiting_is_current(&child_job)); assert!(!authority.repair_waiting_is_current(&parent_job)); drop(authority);
    for _ in 0..512 { app.update(); if app.world().resource::<ResidencyAuthority>().state(&child_key) == Some(PayloadResidencyState::CpuReady) { break; } thread::sleep(Duration::from_millis(1)); }
    assert_eq!(app.world().resource::<ResidencyAuthority>().state(&child_key), Some(PayloadResidencyState::CpuReady)); let activation = SceneCacheStore::new(d.path()).load_activation(child)?.expect("child publication"); assert!(activation.index.entries.iter().any(|entry| entry.address.scene_id == child && entry.geometry.is_some()));
    let mut assets = Assets::<Mesh>::default(); let uploaded = app.world_mut().resource_mut::<ResidencyAuthority>().pump_uploads(&mut assets, Some(usize::MAX)); assert_eq!(uploaded, vec![child_key]); assert_eq!(app.world().resource::<ResidencyAuthority>().state(&child_key), Some(PayloadResidencyState::GpuResident)); Ok(())
}

#[test]
fn deselection_and_reset_release_only_selected_reason() {
    let scene_id = SceneId::new_v4();
    let first = SceneAnchor::active_session("/World/First");
    let second = SceneAnchor::active_session("/World/Second");
    let mut selection = SelectedTargets::default();
    selection
        .add_many(vec![first.clone(), second.clone()], Some(first.clone()))
        .expect("selection is valid");
    let mut app = selection_app(
        selection,
        SceneAnchorIndex::from_test_entities(vec![
            (first.clone(), Entity::from_bits(1)),
            (second.clone(), Entity::from_bits(2)),
        ]),
        presentation(scene_id, 3, vec![entry(scene_id, &first.prim_path, 4)]),
    );
    app.update();
    let key = *app
        .world()
        .resource::<SelectionResidencyState>()
        .selected
        .get(&first)
        .expect("first target maps to payload");
    app.world_mut()
        .resource_mut::<ResidencyAuthority>()
        .request_reason(key, ResidencyReason::CameraNear, 3, 1, 1);
    app.world_mut()
        .resource_mut::<SelectedTargets>()
        .remove(&first)
        .expect("first target removal is valid");
    app.update();
    let reasons = app
        .world()
        .resource::<ResidencyAuthority>()
        .reasons(&key)
        .expect("camera reason keeps record alive");
    assert!(!reasons.contains(&ResidencyReason::Selected));
    assert!(reasons.contains(&ResidencyReason::CameraNear));
    let reset_key = ScenePayloadKey {
        scene_id,
        blob_hash: digest(21),
    };
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene_id, 3, Vec::new());
    assert!(authority.request_reason(reset_key, ResidencyReason::Selected, 3, 1, 1));
    assert!(authority.request_reason(reset_key, ResidencyReason::CameraNear, 3, 1, 1));
    let state = SelectionResidencyState {
        generation: Some(3),
        selected: HashMap::from([(SceneAnchor::active_session("/World/Reset"), reset_key)]),
        ..Default::default()
    };
    let mut world = World::new();
    world.insert_resource(authority);
    world.insert_resource(state);
    release_selected_residency(&mut world);
    let reasons = world
        .resource::<ResidencyAuthority>()
        .reasons(&reset_key)
        .expect("other reason remains");
    assert!(!reasons.contains(&ResidencyReason::Selected));
    assert!(reasons.contains(&ResidencyReason::CameraNear));
    assert!(
        world
            .resource::<SelectionResidencyState>()
            .selected
            .is_empty()
    );
}
#[test]
#[rustfmt::skip]
fn selection_queue_and_stale_work_remain_bounded() {
    let scene_id = SceneId::new_v4(); let later = SceneAnchor::active_session("/World/ZLaterValid");
    let mut targets = (0..512).map(|index| SceneAnchor::active_session(format!("/World/Selected{index}"))).collect::<Vec<_>>();
    targets.push(later.clone());
    let mut selection = SelectedTargets::default(); selection.add_many(targets.clone(), targets.first().cloned()).expect("large selection is valid");
    let index = SceneAnchorIndex::from_test_entity(later.clone(), Entity::from_bits(1));
    let presentation = presentation(scene_id, 1, vec![entry(scene_id, &later.prim_path, 77)]);
    let mut state = SelectionResidencyState { generation: Some(1), selection_revision: Some(selection.revision()), scene_revision: Some(index.revision()), lookup: build_lookup(&presentation, None), ..Default::default() };
    let mut authority = ResidencyAuthority::default(); authority.install_scene(scene_id, 1, Vec::new());
    fill_selection_queue(&selection, &mut state, 1); assert_eq!(SELECTION_RESIDENCY_BATCH, 512); assert_eq!(state.pending_len(), SELECTION_RESIDENCY_QUEUE_CAPACITY); assert_eq!(state.selection_cursor, 512);
    for update in 0..2 {
        advance_selection_queue(&selection, &index, scene_id, &mut authority, &mut state, 1);
        assert!(state.last_batch_work() <= SELECTION_RESIDENCY_BATCH);
        assert!(state.pending_len() <= SELECTION_RESIDENCY_QUEUE_CAPACITY);
        if state.selected.contains_key(&later) {
            assert_eq!(update, 1);
            break;
        }
        fill_selection_queue(&selection, &mut state, 1);
    }
    assert!(state.selected.contains_key(&later));
    let scene_id = SceneId::new_v4(); let (selection, target, index, mut state) = single_target_fixture(scene_id, "/World/Stale", 41);
    state.pending.push_back(PendingSelection { target: target.clone(), selection_revision: selection.revision().saturating_sub(1), generation: 1 }); state.queued.insert(target);
    let mut authority = ResidencyAuthority::default(); authority.install_scene(scene_id, 1, Vec::new());
    advance_selection_queue(&selection, &index, scene_id, &mut authority, &mut state, 1);
    assert!(state.selected.is_empty());
    assert!(state.pending.is_empty());
}

#[test]
fn backpressured_selection_is_attempted_once_then_deferred() {
    let scene_id = SceneId::new_v4();
    let (selection, _target, index, mut state) =
        single_target_fixture(scene_id, "/World/Backpressured", 31);
    fill_selection_queue(&selection, &mut state, 1);
    let mut authority = ResidencyAuthority::default();
    authority.install_scene(scene_id, 1, Vec::new());
    for value in 0..256 {
        assert!(authority.request_reason(
            ScenePayloadKey {
                scene_id,
                blob_hash: digest(1000 + value)
            },
            ResidencyReason::CameraNear,
            1,
            1,
            1
        ));
    }
    advance_selection_queue(&selection, &index, scene_id, &mut authority, &mut state, 1);
    assert_eq!(state.last_batch_work(), 1);
    assert_eq!(state.pending_len(), 0);
    fill_selection_queue(&selection, &mut state, 1);
    advance_selection_queue(&selection, &index, scene_id, &mut authority, &mut state, 1);
    assert_eq!(state.last_batch_work(), 1);
    assert_eq!(state.pending_len(), 0);
}
