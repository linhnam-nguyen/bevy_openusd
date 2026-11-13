use bevy::asset::AssetPlugin;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};

use super::*;
use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::scene::{HoveredTarget, SectionBoxState};

fn clipping_test_app() -> App {
    let mut app = App::new();
    let mut section_box = SectionBoxState::default();
    section_box.enabled = true;
    section_box.visible = true;
    app.add_plugins(MinimalPlugins)
        .add_plugins(AssetPlugin::default())
        .init_asset::<StandardMaterial>()
        .init_asset::<SectionClipMaterial>()
        .insert_resource(section_box)
        .insert_resource(ViewerSettingsState::default())
        .insert_resource(DisplayToggles::default())
        .insert_resource(HoveredTarget::default())
        .insert_resource(SceneAnchorIndex::default())
        .insert_resource(SectionClipProjectionState::default())
        .insert_resource(SectionClipDiagnostics::default())
        .add_systems(Update, sync_section_box_clipping);
    app
}

#[test]
fn clip_uniform_uses_one_shared_box_space_transform() {
    let transform =
        Transform::from_translation(Vec3::new(2.0, 3.0, 4.0)).with_scale(Vec3::splat(6.0));
    let uniform = SectionClipUniform {
        world_to_box: transform.to_matrix().inverse(),
        enabled: 1,
        _padding: Vec3::ZERO,
    };

    assert_eq!(uniform.enabled, 1);
    assert!(
        uniform
            .world_to_box
            .transform_point3(transform.translation)
            .abs_diff_eq(Vec3::ZERO, 0.0001)
    );
}

#[test]
fn extension_keeps_the_standard_material_as_the_base_route() {
    let base = StandardMaterial {
        base_color: Color::srgb(0.2, 0.4, 0.8),
        perceptual_roughness: 0.35,
        ..default()
    };
    let extended = ExtendedMaterial {
        base: base.clone(),
        extension: SectionClipExtension::default(),
    };

    assert_eq!(extended.base.base_color, base.base_color);
    assert_eq!(
        extended.base.perceptual_roughness,
        base.perceptual_roughness
    );
}

#[test]
fn composition_preserves_uniform_and_selection_routes() {
    let mut materials = Assets::<StandardMaterial>::default();
    let authored = materials.add(StandardMaterial::default());
    let uniform = materials.add(StandardMaterial::default());
    let selection = materials.add(StandardMaterial::default());
    let original = OriginalRenderMaterial(authored.clone());
    let selection_base = SelectionBaseMaterial(uniform.clone());

    let composed = support::compose_clipped_route(
        selection.clone(),
        Some(&original),
        Some(&selection_base),
        true,
        true,
        Some(&selection),
        true,
        false,
        None,
        false,
        RenderMode::UniformColor,
        Some(&uniform),
    );

    assert_eq!(composed.route, selection);
    assert_eq!(composed.original, Some(authored));
    assert_eq!(composed.selection_base, Some(uniform));
    assert!(composed.selection_override);
}

#[test]
fn composition_restores_authored_route_after_shaded_selection_release() {
    let mut materials = Assets::<StandardMaterial>::default();
    let authored = materials.add(StandardMaterial::default());
    let selection = materials.add(StandardMaterial::default());
    let original = OriginalRenderMaterial(authored.clone());
    let selection_base = SelectionBaseMaterial(authored.clone());

    let composed = support::compose_clipped_route(
        selection,
        Some(&original),
        Some(&selection_base),
        true,
        false,
        None,
        false,
        false,
        None,
        false,
        RenderMode::Shaded,
        None,
    );

    assert_eq!(composed.route, authored);
    assert_eq!(composed.original, None);
    assert_eq!(composed.selection_base, None);
    assert!(!composed.selection_override);
}

#[test]
fn stale_uniform_route_is_reconciled_before_shaded_clip_removal() {
    let mut materials = Assets::<StandardMaterial>::default();
    let authored = materials.add(StandardMaterial::default());
    let stale_uniform = materials.add(StandardMaterial::default());
    let original = OriginalRenderMaterial(authored.clone());

    let composed = support::compose_clipped_route(
        stale_uniform,
        Some(&original),
        None,
        false,
        false,
        None,
        false,
        false,
        None,
        false,
        RenderMode::Shaded,
        None,
    );

    assert_eq!(composed.route, authored);
    assert_eq!(composed.original, None);
    assert_eq!(composed.selection_base, None);
    assert!(!composed.selection_override);
}

#[test]
fn successful_clip_clears_recovered_unsupported_diagnostic() {
    let mut app = clipping_test_app();
    let entity = app.world_mut().spawn(Mesh3d(Handle::default())).id();
    app.world_mut()
        .resource_mut::<SectionClipProjectionState>()
        .selected_meshes
        .insert(entity);
    let mut projection = app.world_mut().resource_mut::<SectionClipProjectionState>();
    projection.last_targets = Some(Vec::new());
    projection.last_scene_revision = Some(0);

    app.update();
    assert!(
        app.world()
            .resource::<SectionClipDiagnostics>()
            .unsupported_entities
            .contains(&entity)
    );

    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    app.world_mut()
        .entity_mut(entity)
        .insert(MeshMaterial3d(material));
    app.world_mut().resource_mut::<SectionBoxState>().revision = 1;
    app.update();

    let diagnostics = app.world().resource::<SectionClipDiagnostics>();
    assert!(!diagnostics.unsupported_entities.contains(&entity));
    assert!(
        app.world()
            .get::<MeshMaterial3d<SectionClipMaterial>>(entity)
            .is_some()
    );
}

#[test]
fn successful_clip_clears_recovered_missing_material_diagnostic() {
    let mut app = clipping_test_app();
    let missing_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .reserve_handle();
    let entity = app
        .world_mut()
        .spawn((
            Mesh3d(Handle::default()),
            MeshMaterial3d(missing_material.clone()),
        ))
        .id();
    app.world_mut()
        .resource_mut::<SectionClipProjectionState>()
        .selected_meshes
        .insert(entity);
    let mut projection = app.world_mut().resource_mut::<SectionClipProjectionState>();
    projection.last_targets = Some(Vec::new());
    projection.last_scene_revision = Some(0);

    app.update();
    assert!(
        app.world()
            .resource::<SectionClipDiagnostics>()
            .missing_material_entities
            .contains(&entity)
    );

    app.world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .insert(missing_material.id(), StandardMaterial::default())
        .expect("reserved material handle must accept its first asset");
    app.world_mut().resource_mut::<SectionBoxState>().revision = 1;
    app.update();

    let diagnostics = app.world().resource::<SectionClipDiagnostics>();
    assert!(!diagnostics.missing_material_entities.contains(&entity));
    assert!(
        app.world()
            .get::<MeshMaterial3d<SectionClipMaterial>>(entity)
            .is_some()
    );
}
