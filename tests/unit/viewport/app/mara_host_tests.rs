use std::collections::HashSet;

use bevy::prelude::*;
use bevy::render::settings::WgpuLimits;
use mara::ui::mara_core::icons::icon_glyph;
use viewport_protocol::{OverlayKind, ViewportCommand};

use super::panels::{MARA_ICONS, PanelAction, apply_panel_action};
use super::{MaraHostUiState, UsdMaraApp};
use crate::api::ViewportCommandInbox;
use crate::viewport::rendering::gpu_limits::GpuLimitNegotiation;

#[test]
fn mara_host_negotiates_gpu_storage_limits_for_adapter() {
    let supported = WgpuLimits {
        max_storage_buffers_per_shader_stage: 24,
        ..WgpuLimits::default()
    };
    let mut requested = WgpuLimits {
        max_storage_buffers_per_shader_stage: 8,
        ..WgpuLimits::default()
    };
    let result = UsdMaraApp::negotiate_gpu_limits(&supported, &mut requested);
    assert_eq!(result, GpuLimitNegotiation::Supported(16));
    assert_eq!(requested.max_storage_buffers_per_shader_stage, 16);
}

#[test]
fn mara_toolbar_and_panel_icons_resolve_to_distinct_glyphs() {
    let mut seen = HashSet::new();
    for name in MARA_ICONS {
        let glyph = icon_glyph(name)
            .unwrap_or_else(|| panic!("missing Fluent icon glyph for Mara host: {name}"));
        assert!(seen.insert(glyph), "duplicate Mara icon glyph for {name}");
    }
}

#[test]
fn mara_panel_actions_dispatch_through_single_viewport_command_inbox() {
    let mut world = World::new();
    world.init_resource::<ViewportCommandInbox>();
    world.init_resource::<crate::api::ViewportReadModelState>();
    world.init_resource::<crate::viewport::navigation::camera::CameraBookmarks>();
    world.init_resource::<crate::viewport::navigation::camera::CameraMount>();
    world.init_resource::<crate::viewport::navigation::camera::FlyTo>();
    world.init_resource::<crate::semantic::viewport::SemanticDiffState>();

    let mut ui_state = MaraHostUiState::default();
    apply_panel_action(
        PanelAction::Command(ViewportCommand::SetOverlay {
            overlay: OverlayKind::GroundGrid,
            enabled: false,
        }),
        &mut world,
        &mut ui_state,
    );

    let envelope = world
        .resource_mut::<ViewportCommandInbox>()
        .pop()
        .expect("Mara panel action pushes onto shared ViewportCommandInbox");
    assert!(matches!(
        envelope.command,
        ViewportCommand::SetOverlay {
            overlay: OverlayKind::GroundGrid,
            enabled: false
        }
    ));
}

#[test]
fn mara_materials_panel_is_read_only_and_preserves_render_assets() {
    let mut world = World::new();
    world.init_resource::<ViewportCommandInbox>();
    world.init_resource::<crate::api::ViewportReadModelState>();
    world.init_resource::<crate::viewport::navigation::camera::CameraBookmarks>();
    world.init_resource::<crate::viewport::navigation::camera::CameraMount>();
    world.init_resource::<crate::viewport::navigation::camera::FlyTo>();
    world.init_resource::<crate::semantic::viewport::SemanticDiffState>();

    let mut materials = Assets::<StandardMaterial>::default();
    let handle_a = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(0.3, 0.6, 0.8),
        perceptual_roughness: 0.45,
        metallic: 0.2,
        ..Default::default()
    });
    let handle_b = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(0.1, 0.2, 0.3),
        perceptual_roughness: 0.8,
        metallic: 0.0,
        ..Default::default()
    });
    let id_a = handle_a.id();
    world.insert_resource(materials);

    world.spawn((
        usd_bevy::UsdPrimRef {
            path: "/World/MeshA".to_owned(),
        },
        bevy::pbr::MeshMaterial3d(handle_a),
    ));
    world.spawn((
        usd_bevy::UsdPrimRef {
            path: "/World/MeshB".to_owned(),
        },
        bevy::pbr::MeshMaterial3d(handle_b),
    ));

    let mut ui_state = MaraHostUiState::default();
    let snap = super::panels::materials::snapshot(&world, &ui_state.pages);
    assert_eq!(snap.entries.len(), 2);
    let entry_a = snap
        .entries
        .iter()
        .find(|entry| entry.id == id_a)
        .expect("bound material A present in read-only snapshot");
    assert!((entry_a.base_color[0] - 0.3).abs() < 1e-5);
    assert!((entry_a.base_color[1] - 0.6).abs() < 1e-5);
    assert!((entry_a.base_color[2] - 0.8).abs() < 1e-5);
    assert!((entry_a.roughness - 0.45).abs() < 1e-5);
    assert!((entry_a.metallic - 0.2).abs() < 1e-5);

    // Save/Undo/Redo/Reload commands continue to route through ViewportCommandInbox
    // while Assets<StandardMaterial> remains unmutated.
    for cmd in [
        ViewportCommand::SaveStage,
        ViewportCommand::UndoEditor,
        ViewportCommand::RedoEditor,
        ViewportCommand::ReloadSession,
    ] {
        apply_panel_action(PanelAction::Command(cmd), &mut world, &mut ui_state);
    }
    let mut inbox = world.resource_mut::<ViewportCommandInbox>();
    assert!(matches!(inbox.pop().unwrap().command, ViewportCommand::SaveStage));
    assert!(matches!(inbox.pop().unwrap().command, ViewportCommand::UndoEditor));
    assert!(matches!(inbox.pop().unwrap().command, ViewportCommand::RedoEditor));
    assert!(matches!(inbox.pop().unwrap().command, ViewportCommand::ReloadSession));
    assert!(inbox.pop().is_none());
}

#[test]
fn mara_page_window_handles_empty_boundaries_shrinking_and_large_counts() {
    use super::panels::bim::{CATALOGUE_PAGE_SIZE, PROPERTY_GROUP_PAGE_SIZE};
    use super::panels::materials::MATERIALS_PAGE_SIZE;
    use super::panels::page_window;

    let empty = page_window(0, 0, CATALOGUE_PAGE_SIZE);
    assert_eq!(empty.total_pages, 0);
    assert_eq!(empty.start, 0);
    assert_eq!(empty.end, 0);
    assert_eq!(empty.range_label(), "0 of 0");

    // Exact page boundaries (12 / 16 / 24): single page, no extra page.
    assert_eq!(page_window(12, 0, CATALOGUE_PAGE_SIZE).total_pages, 1);
    assert_eq!(page_window(16, 0, PROPERTY_GROUP_PAGE_SIZE).total_pages, 1);
    assert_eq!(page_window(24, 0, MATERIALS_PAGE_SIZE).total_pages, 1);

    // 1 beyond boundary (13 / 17 / 25): second page reaches final item cleanly.
    let cat_13 = page_window(13, 1, CATALOGUE_PAGE_SIZE);
    assert_eq!(cat_13.total_pages, 2);
    assert_eq!((cat_13.start, cat_13.end), (12, 13));
    assert_eq!(cat_13.range_label(), "13–13 of 13");

    let prop_17 = page_window(17, 1, PROPERTY_GROUP_PAGE_SIZE);
    assert_eq!(prop_17.total_pages, 2);
    assert_eq!((prop_17.start, prop_17.end), (16, 17));
    assert_eq!(prop_17.range_label(), "17–17 of 17");

    let mat_25 = page_window(25, 1, MATERIALS_PAGE_SIZE);
    assert_eq!(mat_25.total_pages, 2);
    assert_eq!((mat_25.start, mat_25.end), (24, 25));
    assert_eq!(mat_25.range_label(), "25–25 of 25");

    // Shrinking list clamps out-of-bounds desired page without panic.
    let shrunk = page_window(3, 8, CATALOGUE_PAGE_SIZE);
    assert_eq!(shrunk.page, 0);
    assert_eq!((shrunk.start, shrunk.end), (0, 3));
    assert_eq!(shrunk.range_label(), "1–3 of 3");

    // Large and extreme counts never overflow and bound per-frame slice size.
    let large_mat = page_window(1_000, 41, MATERIALS_PAGE_SIZE);
    assert_eq!(large_mat.total_pages, 42);
    assert_eq!((large_mat.start, large_mat.end), (984, 1_000));
    assert!(large_mat.end - large_mat.start <= MATERIALS_PAGE_SIZE);

    let max_win = page_window(usize::MAX, usize::MAX, MATERIALS_PAGE_SIZE);
    assert_eq!(max_win.end, usize::MAX);
    assert!(max_win.end - max_win.start <= MATERIALS_PAGE_SIZE);
}

#[test]
fn mara_ui_pagination_actions_update_only_ui_state_and_reset_on_context_change() {
    use viewport_protocol::{
        BimPropertyGroupId, SceneAnchor, StageLoadState, ViewportEvent, ViewportEventEnvelope,
        ViewportReadModel,
    };

    let mut world = World::new();
    world.init_resource::<ViewportCommandInbox>();
    world.init_resource::<crate::api::ViewportReadModelState>();
    world.init_resource::<crate::viewport::navigation::camera::CameraBookmarks>();
    world.init_resource::<crate::viewport::navigation::camera::CameraMount>();
    world.init_resource::<crate::viewport::navigation::camera::FlyTo>();
    world.init_resource::<crate::semantic::viewport::SemanticDiffState>();

    let a = SceneAnchor::active_session("/World/A");
    let b = SceneAnchor::active_session("/World/B");
    let mut snap_a = ViewportReadModel::unloaded("stage_a.usda");
    snap_a.stage.loaded = true;
    snap_a.stage.activation_generation = 1;
    snap_a.selection.primary = Some(a.clone());
    snap_a.selection.targets = vec![a];
    snap_a.selection_revision = 1;

    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            None,
            ViewportEvent::Snapshot {
                state: Box::new(snap_a.clone()),
            },
        ));

    let mut ui_state = MaraHostUiState::default();
    ui_state.pages.sync_contexts(&world);

    apply_panel_action(PanelAction::SetCataloguePage(2), &mut world, &mut ui_state);
    apply_panel_action(
        PanelAction::SetPropertyPage {
            group_id: BimPropertyGroupId::Instance,
            page: 3,
        },
        &mut world,
        &mut ui_state,
    );
    apply_panel_action(PanelAction::SetMaterialPage(4), &mut world, &mut ui_state);

    // UI-only actions must never push to ViewportCommandInbox.
    assert!(world.resource_mut::<ViewportCommandInbox>().pop().is_none());
    assert_eq!(ui_state.pages.catalogue_page, 2);
    assert_eq!(
        ui_state.pages.property_pages.get(&BimPropertyGroupId::Instance),
        Some(&3)
    );
    assert_eq!(
        ui_state.pages.property_pages.get(&BimPropertyGroupId::Semantic),
        None
    );
    assert_eq!(ui_state.pages.material_page, 4);

    // Selection change resets only property_pages, keeping stage-scoped catalogue/material pages.
    let mut snap_b = snap_a;
    snap_b.selection.primary = Some(b.clone());
    snap_b.selection.targets = vec![b];
    snap_b.selection_revision = 2;
    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            None,
            ViewportEvent::Snapshot {
                state: Box::new(snap_b),
            },
        ));
    ui_state.pages.sync_contexts(&world);
    assert!(ui_state.pages.property_pages.is_empty());
    assert_eq!(ui_state.pages.catalogue_page, 2);
    assert_eq!(ui_state.pages.material_page, 4);

    // Stage transition resets all pagers.
    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            None,
            ViewportEvent::StageLoadStateChanged {
                state: StageLoadState::Loading,
            },
        ));
    ui_state.pages.sync_contexts(&world);
    assert_eq!(ui_state.pages.catalogue_page, 0);
    assert!(ui_state.pages.property_pages.is_empty());
    assert_eq!(ui_state.pages.material_page, 0);
}

#[test]
fn mara_bim_request_dispatch_binds_request_id_and_reflects_correlated_states() {
    use super::panels::bim::{self, BimPropertiesViewState};
    use viewport_protocol::{
        BimPropertiesPage, BimPropertyGroupId, BimPropertyGroupReadModel, BimPropertyReadModel,
        CommonValue, SceneAnchor, ViewportEvent, ViewportEventEnvelope, ViewportReadModel,
    };

    let mut world = World::new();
    world.init_resource::<ViewportCommandInbox>();
    world.init_resource::<crate::api::ViewportReadModelState>();
    world.init_resource::<crate::viewport::rendering::scene::ActiveHierarchyProvider>();
    world.init_resource::<crate::viewport::navigation::camera::CameraBookmarks>();
    world.init_resource::<crate::viewport::navigation::camera::CameraMount>();
    world.init_resource::<crate::viewport::navigation::camera::FlyTo>();
    world.init_resource::<crate::semantic::viewport::SemanticDiffState>();

    let mut ui_state = MaraHostUiState::default();

    // Loaded stage with empty selection -> local rejection, no command queued.
    let mut unloaded_sel = ViewportReadModel::unloaded("s.usda");
    unloaded_sel.stage.loaded = true;
    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            None,
            ViewportEvent::Snapshot {
                state: Box::new(unloaded_sel.clone()),
            },
        ));
    apply_panel_action(
        PanelAction::Command(ViewportCommand::RequestBimProperties),
        &mut world,
        &mut ui_state,
    );
    assert!(world.resource_mut::<ViewportCommandInbox>().pop().is_none());
    assert!(matches!(
        bim::snapshot(&world, &ui_state.pages).properties,
        BimPropertiesViewState::NoSelection
    ));

    // Loaded stage with selection -> dispatches 1 command, binds request ID, shows Loading(1/2) then Ready.
    let a = SceneAnchor::active_session("/World/A");
    let mut with_sel = unloaded_sel;
    with_sel.selection.primary = Some(a.clone());
    with_sel.selection.targets = vec![a.clone()];
    with_sel.selection_revision = 1;
    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            None,
            ViewportEvent::Snapshot {
                state: Box::new(with_sel),
            },
        ));

    apply_panel_action(
        PanelAction::Command(ViewportCommand::RequestBimProperties),
        &mut world,
        &mut ui_state,
    );
    let sent = world
        .resource_mut::<ViewportCommandInbox>()
        .pop()
        .expect("RequestBimProperties dispatched");
    assert!(matches!(sent.command, ViewportCommand::RequestBimProperties));
    assert!(matches!(
        bim::snapshot(&world, &ui_state.pages).properties,
        BimPropertiesViewState::Loading {
            received_pages: 0,
            total_pages: 0
        }
    ));

    let make_prop = |k: &str| BimPropertyReadModel {
        key: k.to_owned(),
        label: k.to_owned(),
        scope: usd_model::BimPropertyScope::Instance,
        group_id: BimPropertyGroupId::Instance,
        value: CommonValue::Multiple,
        target_values: Vec::new(),
        measurement: None,
        units: Vec::new(),
        current_display_unit: None,
        editable: false,
    };

    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            Some(sent.request_id.clone()),
            ViewportEvent::BimPropertiesPage {
                page: BimPropertiesPage {
                    selection_revision: 1,
                    page_index: 0,
                    page_count: 2,
                    total_properties: 2,
                    targets: vec![a],
                    groups: vec![BimPropertyGroupReadModel {
                        id: BimPropertyGroupId::Instance,
                        name: "Instance".to_owned(),
                        editable_group: false,
                        properties: vec![make_prop("P1")],
                    }],
                    diff: None,
                },
            },
        ));
    assert!(matches!(
        bim::snapshot(&world, &ui_state.pages).properties,
        BimPropertiesViewState::Loading {
            received_pages: 1,
            total_pages: 2
        }
    ));

    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            Some(sent.request_id),
            ViewportEvent::BimPropertiesPage {
                page: BimPropertiesPage {
                    selection_revision: 1,
                    page_index: 1,
                    page_count: 2,
                    total_properties: 2,
                    targets: Vec::new(),
                    groups: vec![BimPropertyGroupReadModel {
                        id: BimPropertyGroupId::Instance,
                        name: "Instance".to_owned(),
                        editable_group: false,
                        properties: vec![make_prop("P2")],
                    }],
                    diff: None,
                },
            },
        ));
    match bim::snapshot(&world, &ui_state.pages).properties {
        BimPropertiesViewState::Ready(model) => {
            assert_eq!(model.groups.len(), 1);
            assert_eq!(model.groups[0].properties.len(), 2);
            assert_eq!(model.groups[0].properties[0].value, CommonValue::Multiple);
        }
        _ => panic!("expected Ready assembled BIM properties"),
    }
}

#[test]
fn mara_host_flushes_coalesced_catalogue_refresh_once_with_correlated_request_id() {
    use super::panels::bim::{self, BimCatalogueViewState, BimPropertiesViewState};
    use super::panels::flush_pending_catalogue_refresh;
    use viewport_protocol::{
        BimClassificationFieldCatalogue, BimClassificationFieldDescriptor, BimFieldKey,
        BimPropertiesReadModel, SceneAnchor, ViewportEvent, ViewportEventEnvelope,
        ViewportReadModel,
    };

    let mut world = World::new();
    world.init_resource::<ViewportCommandInbox>();
    world.init_resource::<crate::api::ViewportReadModelState>();
    world.init_resource::<crate::viewport::rendering::scene::ActiveHierarchyProvider>();

    let a = SceneAnchor::active_session("/World/A");
    let mut snap = ViewportReadModel::unloaded("stage_a.usda");
    snap.stage.loaded = true;
    snap.stage.activation_generation = 1;
    snap.selection.primary = Some(a.clone());
    snap.selection.targets = vec![a.clone()];
    snap.selection_revision = 4;
    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            None,
            ViewportEvent::Snapshot {
                state: Box::new(snap),
            },
        ));

    // Burst of anonymous catalogue pushes never directly installs a Ready catalogue.
    let anon_cat = BimClassificationFieldCatalogue {
        semantic_revision: 3,
        fields: vec![BimClassificationFieldDescriptor::new(
            BimFieldKey::property("Category"),
            "Category",
            usd_model::BimPropertyScope::Instance,
        )],
    };
    for _ in 0..10 {
        world
            .resource_mut::<crate::api::ViewportReadModelState>()
            .apply(&ViewportEventEnvelope::new(
                None,
                ViewportEvent::BimClassificationFieldCatalogueChanged {
                    catalogue: anon_cat.clone(),
                },
            ));
    }
    let ui_state = MaraHostUiState::default();
    assert!(matches!(
        bim::snapshot(&world, &ui_state.pages).catalogue,
        BimCatalogueViewState::NotRequested
    ));

    // Host flushes exactly ONE correlated RequestBimClassificationFieldCatalogue command.
    flush_pending_catalogue_refresh(&mut world);
    flush_pending_catalogue_refresh(&mut world);
    let mut inbox = world.resource_mut::<ViewportCommandInbox>();
    let sent = inbox
        .pop()
        .expect("single correlated catalogue refresh command dispatched");
    assert!(matches!(
        sent.command,
        ViewportCommand::RequestBimClassificationFieldCatalogue {
            known_revision: None
        }
    ));
    assert!(inbox.pop().is_none());
    drop(inbox);

    assert!(matches!(
        bim::snapshot(&world, &ui_state.pages).catalogue,
        BimCatalogueViewState::Loading {
            received_pages: 0,
            total_pages: 0
        }
    ));

    // Correlated response installs Ready catalogue in BIM panel snapshot.
    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            Some(sent.request_id),
            ViewportEvent::BimClassificationFieldCatalogueChanged {
                catalogue: anon_cat.clone(),
            },
        ));
    match bim::snapshot(&world, &ui_state.pages).catalogue {
        BimCatalogueViewState::Ready(cat) => assert_eq!(cat, anon_cat),
        _ => panic!("expected Ready catalogue after correlated reply"),
    }

    // Mismatched current-ID BimPropertiesRead surfaces Error in BIM panel snapshot (not Loading).
    let mut ui_state_mut = ui_state;
    apply_panel_action(
        PanelAction::Command(ViewportCommand::RequestBimProperties),
        &mut world,
        &mut ui_state_mut,
    );
    let prop_req = world
        .resource_mut::<ViewportCommandInbox>()
        .pop()
        .expect("RequestBimProperties dispatched");
    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            Some(prop_req.request_id),
            ViewportEvent::BimPropertiesRead {
                properties: BimPropertiesReadModel {
                    targets: vec![a],
                    selection_revision: 99,
                    groups: Vec::new(),
                },
                diff: None,
            },
        ));
    assert!(matches!(
        bim::snapshot(&world, &ui_state_mut.pages).properties,
        BimPropertiesViewState::Error(_)
    ));
}

#[test]
fn mara_host_preserves_not_requested_catalogue_when_command_inbox_absent() {
    use super::panels::bim::{self, BimCatalogueViewState};
    use super::panels::flush_pending_catalogue_refresh;
    use viewport_protocol::{
        BimClassificationFieldCatalogue, ViewportEvent, ViewportEventEnvelope, ViewportReadModel,
    };

    let mut world = World::new();
    world.init_resource::<crate::api::ViewportReadModelState>();
    world.init_resource::<crate::viewport::rendering::scene::ActiveHierarchyProvider>();

    let mut snap = ViewportReadModel::unloaded("standalone.usda");
    snap.stage.loaded = true;
    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            None,
            ViewportEvent::Snapshot {
                state: Box::new(snap),
            },
        ));
    world
        .resource_mut::<crate::api::ViewportReadModelState>()
        .apply(&ViewportEventEnvelope::new(
            None,
            ViewportEvent::BimClassificationFieldCatalogueChanged {
                catalogue: BimClassificationFieldCatalogue::default(),
            },
        ));

    flush_pending_catalogue_refresh(&mut world);
    let ui_state = MaraHostUiState::default();
    assert!(matches!(
        bim::snapshot(&world, &ui_state.pages).catalogue,
        BimCatalogueViewState::NotRequested
    ));
    assert_eq!(
        world
            .resource::<crate::api::ViewportReadModelState>()
            .pending_catalogue_refresh(),
        None
    );
}
