use std::time::Instant;

use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use usd_bevy::{UsdLocalExtent, UsdPrimRef};
use viewport_protocol::{
    ClientCommand, ClientCommandEnvelope, SceneAnchor, SelectionReadModel, ViewportCommand,
    decode_client_json_line, encode_client_json_line,
};

use super::super::ViewerSettingsState;
use super::support::command_test_app;
use crate::viewport::api::bridge::commands::apply_viewport_commands;
use crate::viewport::api::scene_index::refresh_scene_anchor_index;
use crate::viewport::api::{SceneAnchorIndex, ViewportCommandInbox};
use crate::viewport::scene::HoveredTarget;
use crate::viewport::scene::{
    HoverColorMaterial, SectionBoxState, SelectedTargets, SelectionColorMaterial,
    SelectionColorOverride, SelectionColorOverrideState, SelectionOutline, SelectionOutlineState,
    aggregate_selection_bounds, selected_renderable_entities, sync_section_box_state,
    sync_selection_color_overrides, sync_selection_outlines,
};
use crate::viewport::session::{Spawned, StageInfo};
use selection_profile_support::repeat_selection_updates;

const PROFILE_SIZES: [usize; 6] = [1, 10, 100, 256, 1_000, 5_000];
const PROTOCOL_PROFILE_SIZES: [usize; 8] = [1, 10, 100, 255, 256, 257, 1_000, 5_000];
const ACCEPTED_SCENE_SIZE: usize = 5_000;
const REPEATS: usize = 5;

#[derive(Resource, Default)]
struct ProfileCount(usize);

fn anchor(index: usize) -> SceneAnchor {
    SceneAnchor::active_session(format!("/World/Profile{index:05}"))
}

fn selection(size: usize) -> SelectionReadModel {
    let targets = (0..size).map(anchor).collect::<Vec<_>>();
    SelectionReadModel {
        primary: targets.first().cloned(),
        targets,
    }
}

fn indexed_scene_app(size: usize) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<SelectedTargets>()
        .init_resource::<ViewerSettingsState>()
        .init_resource::<Spawned>()
        .insert_resource(StageInfo {
            path: "fixtures/profile.usda".to_owned(),
            ..default()
        })
        .add_systems(Update, refresh_scene_anchor_index);
    for index in 0..size {
        app.world_mut().spawn((
            UsdPrimRef::new(format!("/World/Profile{index:05}")),
            Mesh3d(Handle::<Mesh>::default()),
            GlobalTransform::from(Transform::from_xyz(index as f32, 0.0, 0.0)),
            UsdLocalExtent {
                min: [-0.5, -0.5, -0.5],
                max: [0.5, 0.5, 0.5],
            },
        ));
    }
    app.world_mut().resource_mut::<Spawned>().0 = true;
    app.update();
    app
}

fn set_selection(app: &mut App, value: SelectionReadModel) {
    app.world_mut()
        .resource_mut::<SelectedTargets>()
        .replace(value)
        .expect("profile selection must be valid");
}

fn median_micros(samples: &mut [u128]) -> u128 {
    samples.sort_unstable();
    samples[samples.len() / 2].max(1)
}

fn profile_protocol() {
    for size in PROTOCOL_PROFILE_SIZES {
        let envelope = ClientCommandEnvelope::new(
            format!("profile-{size}"),
            1,
            ClientCommand::Viewport(ViewportCommand::ReplaceSelection {
                targets: selection(size).targets,
                primary: (size > 0).then(|| anchor(0)),
            }),
        );
        let started = Instant::now();
        let wire = encode_client_json_line(&envelope).expect("profile command must encode");
        let encode_micros = started.elapsed().as_micros();
        let started = Instant::now();
        let decoded = decode_client_json_line(&wire).expect("profile command must decode");
        let decode_micros = started.elapsed().as_micros();
        let started = Instant::now();
        let validation = decoded.validate();
        let validation_micros = started.elapsed().as_micros();
        if size <= viewport_protocol::MAX_SELECTION_TARGETS {
            assert!(validation.is_ok(), "protocol size {size} must be accepted");
        }
        println!(
            "I1.7.1 protocol size={size} wire_bytes={} encode_us={encode_micros} decode_us={decode_micros} validate_us={validation_micros} validation={:?}",
            wire.len(),
            validation
        );
    }
}
fn profile_authority() {
    let mut app = command_test_app();
    app.init_resource::<SceneAnchorIndex>()
        .init_resource::<Spawned>()
        .insert_resource(StageInfo {
            path: "fixtures/profile.usda".to_owned(),
            ..default()
        })
        .add_systems(
            Update,
            refresh_scene_anchor_index.before(apply_viewport_commands),
        );
    for index in 0..ACCEPTED_SCENE_SIZE {
        app.world_mut().spawn((
            UsdPrimRef::new(format!("/World/Profile{index:05}")),
            Mesh3d(Handle::<Mesh>::default()),
        ));
    }
    app.world_mut().resource_mut::<Spawned>().0 = true;
    app.update();
    for size in PROTOCOL_PROFILE_SIZES
        .into_iter()
        .filter(|size| *size <= viewport_protocol::MAX_SELECTION_TARGETS)
    {
        let value = selection(size);
        let mut samples = Vec::with_capacity(REPEATS);
        for _ in 0..REPEATS {
            let started = Instant::now();
            app.world_mut().resource_mut::<ViewportCommandInbox>().send(
                ViewportCommand::ReplaceSelection {
                    targets: value.targets.clone(),
                    primary: value.primary.clone(),
                },
            );
            app.update();
            samples.push(started.elapsed().as_micros());
            assert_eq!(
                app.world().resource::<SelectedTargets>().0.targets.len(),
                size
            );
            app.world_mut()
                .resource_mut::<ViewportCommandInbox>()
                .send(ViewportCommand::SelectTarget { target: None });
            app.update();
        }
        let median = median_micros(&mut samples);
        let maximum = samples.iter().copied().max().unwrap_or_default();
        println!(
            "I1.7.1 authority size={size} command_to_authoritative_us={median} max_update_us={maximum}"
        );
    }
}
fn profile_resolve_roots() {
    fn resolve(
        selection: Res<SelectedTargets>,
        index: Res<SceneAnchorIndex>,
        mut result: ResMut<ProfileCount>,
    ) {
        result.0 = selection
            .0
            .targets
            .iter()
            .filter(|target| index.resolve(target).is_some())
            .count();
    }
    for size in PROFILE_SIZES {
        let mut app = indexed_scene_app(ACCEPTED_SCENE_SIZE);
        app.insert_resource(ProfileCount::default())
            .add_systems(Update, resolve);
        app.update();
        let value = selection(size);
        let (micros, max_update, settle_frames) =
            repeat_selection_updates(&mut app, &value, App::update);
        assert_eq!(app.world().resource::<ProfileCount>().0, size);
        println!(
            "I1.7.1 scene_anchor_resolution size={size} median_us={micros} max_update_us={max_update} settle_max_frames={settle_frames}"
        );
    }
}
fn profile_renderable_resolution() {
    fn resolve(
        selection: Res<SelectedTargets>,
        index: Res<SceneAnchorIndex>,
        renderables: Query<(
            Option<&GlobalTransform>,
            Option<&Children>,
            Option<&Mesh3d>,
            Option<&bevy::camera::primitives::Aabb>,
            Option<&UsdLocalExtent>,
        )>,
        mut result: ResMut<ProfileCount>,
    ) {
        result.0 = selected_renderable_entities(&selection.0.targets, &index, &renderables).len();
    }
    for size in PROFILE_SIZES {
        let mut app = indexed_scene_app(ACCEPTED_SCENE_SIZE);
        app.insert_resource(ProfileCount::default())
            .add_systems(Update, resolve);
        app.update();
        let value = selection(size);
        let (micros, max_update, settle_frames) =
            repeat_selection_updates(&mut app, &value, App::update);
        assert_eq!(app.world().resource::<ProfileCount>().0, size);
        println!(
            "I1.7.1 selected_renderable_resolution size={size} median_us={micros} max_update_us={max_update} settle_max_frames={settle_frames}"
        );
    }
}
fn profile_bounds() {
    fn aggregate(
        selection: Res<SelectedTargets>,
        index: Res<SceneAnchorIndex>,
        renderables: Query<(
            Option<&GlobalTransform>,
            Option<&Children>,
            Option<&Mesh3d>,
            Option<&bevy::camera::primitives::Aabb>,
            Option<&UsdLocalExtent>,
        )>,
        mut result: ResMut<ProfileCount>,
    ) {
        result.0 = usize::from(
            aggregate_selection_bounds(&selection.0.targets, &index, &renderables).is_some(),
        );
    }

    for size in PROFILE_SIZES {
        let mut app = indexed_scene_app(ACCEPTED_SCENE_SIZE);
        app.insert_resource(ProfileCount::default())
            .add_systems(Update, aggregate);
        app.update();
        let value = selection(size);
        let (micros, max_update, settle_frames) =
            repeat_selection_updates(&mut app, &value, App::update);
        assert_eq!(app.world().resource::<ProfileCount>().0, 1);
        println!(
            "I1.7.1 section_box_aggregate_bounds size={size} median_us={micros} max_update_us={max_update} settle_max_frames={settle_frames}"
        );
    }
}
fn profile_outline() {
    for size in PROFILE_SIZES {
        let mut app = indexed_scene_app(ACCEPTED_SCENE_SIZE);
        app.init_resource::<SelectionOutlineState>()
            .add_systems(Update, sync_selection_outlines);
        app.update();
        let value = selection(size);
        let (micros, max_update, settle_frames) =
            repeat_selection_updates(&mut app, &value, App::update);
        let count = app
            .world_mut()
            .query_filtered::<Entity, With<SelectionOutline>>()
            .iter(app.world())
            .count();
        assert_eq!(count, size);
        println!(
            "I1.7.1 selection_boundary_outline size={size} median_us={micros} max_update_us={max_update} settle_max_frames={settle_frames}"
        );
    }
}
fn profile_section_box() {
    for size in PROFILE_SIZES {
        let mut app = indexed_scene_app(ACCEPTED_SCENE_SIZE);
        app.world_mut()
            .resource_mut::<ViewerSettingsState>()
            .set_section_box_enabled(true);
        app.init_resource::<SectionBoxState>()
            .add_systems(Update, sync_section_box_state);
        app.update();
        let value = selection(size);
        let (micros, max_update, settle_frames) =
            repeat_selection_updates(&mut app, &value, App::update);
        assert!(app.world().resource::<SectionBoxState>().visible);
        assert_eq!(
            app.world().resource::<SectionBoxState>().targets.len(),
            size
        );
        let idle_revision = app.world().resource::<SectionBoxState>().revision;
        app.update();
        assert_eq!(
            app.world().resource::<SectionBoxState>().revision,
            idle_revision
        );
        println!(
            "I1.7.1 section_box_reconciliation size={size} median_us={micros} max_update_us={max_update} settle_max_frames={settle_frames}"
        );
    }
}
fn profile_color() {
    for size in PROFILE_SIZES {
        let mut app = indexed_scene_app(ACCEPTED_SCENE_SIZE);
        app.init_resource::<Assets<StandardMaterial>>()
            .init_resource::<SelectionColorOverrideState>()
            .init_resource::<HoveredTarget>();
        let base = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let selection_material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let hover_material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        app.insert_resource(SelectionColorMaterial(selection_material))
            .insert_resource(HoverColorMaterial(hover_material));
        let meshes = app
            .world_mut()
            .query_filtered::<Entity, With<Mesh3d>>()
            .iter(app.world())
            .collect::<Vec<_>>();
        for entity in meshes {
            app.world_mut()
                .entity_mut(entity)
                .insert(MeshMaterial3d(base.clone()));
        }
        app.world_mut()
            .resource_mut::<ViewerSettingsState>()
            .0
            .selection
            .color_change_enabled = true;
        app.add_systems(Update, sync_selection_color_overrides);
        app.update();
        let value = selection(size);
        let (micros, max_update, settle_frames) =
            repeat_selection_updates(&mut app, &value, App::update);
        let count = app
            .world_mut()
            .query_filtered::<Entity, With<SelectionColorOverride>>()
            .iter(app.world())
            .count();
        assert_eq!(count, size);
        println!(
            "I1.7.1 selection_color_material size={size} median_us={micros} max_update_us={max_update} settle_max_frames={settle_frames}"
        );
    }
}
#[test]
#[ignore = "I1.7.1 profile checkpoint; run explicitly with --ignored"]
fn profile_i1_7_1_large_multi_selection_baseline() {
    profile_protocol();
    profile_authority();
    profile_resolve_roots();
    profile_renderable_resolution();
    profile_bounds();
    profile_outline();
    profile_color();
    profile_section_box();
    projection_profile_test::profile_gizmo_reconciliation();
}

#[path = "selection_profile_projection_test.rs"]
mod projection_profile_test;

#[path = "selection_profile_support.rs"]
mod selection_profile_support;

#[path = "selection_projection_cache_test.rs"]
mod projection_cache_test;
