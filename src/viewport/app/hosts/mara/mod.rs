//! Native Mara host adapter (`WindowApp` + `MaraBevyViewport` + shared backend).
//!
//! Mara owns the native OS window, borderless chrome, ribbons, panes, and
//! egui/wgpu context (`dave-gray101/mara`), while embedding a single shared
//! Bevy USD `World` configured via `app::composition`. All commands, edits,
//! selection, BIM, settings, and timeline operations dispatch through the
//! single authoritative `ViewportCommandInbox` / `ViewportReadModelState`.

mod input;
mod panels;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, mpsc};

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use mara::host::{MaraHostCtx, RibbonRail};
use mara::ui::mara_core;
use mara::ui::modules::bevy as mara_bevy;
use mara::window::{AppRunner, CreationContext, ShellBar, ShellEvent, WindowApp};
use mara_core::style::{AccentColor, GlassOpacity, Mode, active_accent};
use mara_core::{RibbonAvoidance, ShellView, WorkspaceStack};
use viewport_protocol::{BimPropertyGroupId, SceneAnchor, ViewportCommand};

use crate::viewport::api::ViewportReadModelState;

use crate::viewport::app::composition;
use crate::viewport::app::launch_options::LaunchOptions;
use crate::viewport::app::scene::spawn_camera_and_ground;
use crate::viewport::camera::{CameraMount, FlyTo};
use crate::viewport::diagnostics::log_capture::LoaderLog;
use crate::viewport::input::HostUiInputCapture;
use crate::viewport::rendering::gpu_limits::{self, GpuLimitNegotiation};
use crate::viewport::stage::LoadRequest;

#[derive(Clone)]
struct MaraLaunchConfig {
    launch_options: LaunchOptions,
    asset_path: String,
    asset_root: PathBuf,
    is_s8: bool,
}

static PENDING_MARA_LAUNCH: Mutex<Option<MaraLaunchConfig>> = Mutex::new(None);

/// Launches the native Mara window host over the shared Bevy USD composition.
pub(crate) fn run(
    launch_options: &LaunchOptions,
    asset_path: &str,
    asset_root: PathBuf,
    is_s8: bool,
) {
    {
        let mut slot = PENDING_MARA_LAUNCH
            .lock()
            .expect("mara launch mutex poisoned");
        *slot = Some(MaraLaunchConfig {
            launch_options: launch_options.clone(),
            asset_path: asset_path.to_owned(),
            asset_root,
            is_s8,
        });
    }

    let title = if asset_path.is_empty() {
        "usdview".to_owned()
    } else {
        format!("usdview — {asset_path}")
    };

    if let Err(error) = AppRunner::new()
        .title(title)
        .size(1400.0, 900.0)
        .run::<UsdMaraApp>()
    {
        eprintln!("usdview (mara host): {error}");
        std::process::exit(1);
    }
}

/// Ensures every `Camera3d` in the embedded Bevy app targets Mara's offscreen
/// `BevyViewportRenderTarget` so `mara_bevy` redirects it to the shared manual
/// GPU texture view without creating a second Bevy window.
fn bind_cameras_to_mara_render_target(
    render_target: Option<Res<mara_bevy::BevyViewportRenderTarget>>,
    mut cameras: Query<&mut RenderTarget, With<Camera3d>>,
) {
    let Some(render_target) = render_target else {
        return;
    };
    for mut target in &mut cameras {
        if matches!(*target, RenderTarget::Window(_)) {
            *target = RenderTarget::from(render_target.0.clone());
        }
    }
}

/// Asynchronous native file-picker request/result so `rfd` never blocks UI frames.
pub(super) enum PendingFileDialog {
    OpenUsd(mpsc::Receiver<Option<PathBuf>>),
    SaveStageAs(mpsc::Receiver<Option<PathBuf>>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BimPropertiesPageContext {
    stage_epoch: u64,
    activation_generation: u64,
    stage_name: String,
    selection_revision: u64,
    targets: Vec<SceneAnchor>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CataloguePageContext {
    stage_epoch: u64,
    activation_generation: u64,
    stage_name: String,
    semantic_revision: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct MaterialsPageContext {
    stage_epoch: u64,
    activation_generation: u64,
    stage_name: String,
}

/// Transient UI pagination state owned by the Mara native host.
#[derive(Default)]
pub(in crate::viewport::app::hosts::mara) struct MaraPageState {
    pub(super) catalogue_page: usize,
    pub(super) property_pages: HashMap<BimPropertyGroupId, usize>,
    pub(super) material_page: usize,
    catalogue_ctx: CataloguePageContext,
    properties_ctx: BimPropertiesPageContext,
    materials_ctx: MaterialsPageContext,
}

impl MaraPageState {
    pub(super) fn sync_contexts(&mut self, world: &World) {
        let rm = world.resource::<ViewportReadModelState>();
        let snap = rm.snapshot();
        let stage_epoch = rm.stage_epoch();
        let (activation_generation, stage_name, selection_revision, targets) = match snap {
            Some(s) if s.stage.loaded => (
                s.stage.activation_generation,
                s.stage.display_name.clone(),
                s.selection_revision,
                s.selection.targets.clone(),
            ),
            _ => (0, String::new(), 0, Vec::new()),
        };
        let semantic_revision = rm
            .bim_classification_catalogue()
            .map(|cat| cat.semantic_revision);

        let next_catalogue_ctx = CataloguePageContext {
            stage_epoch,
            activation_generation,
            stage_name: stage_name.clone(),
            semantic_revision,
        };
        if self.catalogue_ctx != next_catalogue_ctx {
            self.catalogue_ctx = next_catalogue_ctx;
            self.catalogue_page = 0;
        }

        let next_properties_ctx = BimPropertiesPageContext {
            stage_epoch,
            activation_generation,
            stage_name: stage_name.clone(),
            selection_revision,
            targets,
        };
        if self.properties_ctx != next_properties_ctx {
            self.properties_ctx = next_properties_ctx;
            self.property_pages.clear();
        }

        let next_materials_ctx = MaterialsPageContext {
            stage_epoch,
            activation_generation,
            stage_name,
        };
        if self.materials_ctx != next_materials_ctx {
            self.materials_ctx = next_materials_ctx;
            self.material_page = 0;
        }
    }
}

/// Transient UI presentation state owned by the Mara native host.
#[derive(Default)]
pub(super) struct MaraHostUiState {
    pub(super) tree: panels::tree::TreeUiState,
    pub(super) pages: MaraPageState,
    pub(super) pending_dialog: Option<PendingFileDialog>,
    pub(super) last_viewport_drag_pos: Option<egui::Pos2>,
    pub(super) viewport_drag_active: bool,
}

struct UsdMaraApp {
    bevy_view: mara_bevy::MaraBevyViewport,
    workspace: WorkspaceStack,
    ui_state: MaraHostUiState,
    shell_initialized: bool,
}

impl UsdMaraApp {
    /// Negotiates GPU device limits using `gpu_limits::configure` so skinned
    /// and morphed USD meshes receive their required storage buffers within
    /// adapter bounds.
    fn negotiate_gpu_limits(
        supported: &bevy::render::settings::WgpuLimits,
        requested: &mut bevy::render::settings::WgpuLimits,
    ) -> GpuLimitNegotiation {
        gpu_limits::configure(supported, requested)
    }

    fn poll_file_dialogs(&mut self) {
        let Some(pending) = self.ui_state.pending_dialog.take() else {
            return;
        };
        let Some(world) = self.bevy_view.world_mut() else {
            self.ui_state.pending_dialog = Some(pending);
            return;
        };
        match pending {
            PendingFileDialog::OpenUsd(rx) => match rx.try_recv() {
                Ok(Some(path)) => {
                    world.resource_mut::<LoadRequest>().path = Some(path);
                }
                Ok(None) => {}
                Err(mpsc::TryRecvError::Empty) => {
                    self.ui_state.pending_dialog = Some(PendingFileDialog::OpenUsd(rx));
                }
                Err(mpsc::TryRecvError::Disconnected) => {}
            },
            PendingFileDialog::SaveStageAs(rx) => match rx.try_recv() {
                Ok(Some(path)) => {
                    let filename = path.to_string_lossy().into_owned();
                    world
                        .resource_mut::<crate::viewport::api::ViewportCommandInbox>()
                        .send(ViewportCommand::SaveStageAs { filename });
                }
                Ok(None) => {}
                Err(mpsc::TryRecvError::Empty) => {
                    self.ui_state.pending_dialog = Some(PendingFileDialog::SaveStageAs(rx));
                }
                Err(mpsc::TryRecvError::Disconnected) => {}
            },
        }
    }
}

impl WindowApp for UsdMaraApp {
    fn configure_gpu_limits(
        supported: &bevy::render::settings::WgpuLimits,
        requested: &mut bevy::render::settings::WgpuLimits,
    ) {
        let negotiation = Self::negotiate_gpu_limits(supported, requested);
        if let GpuLimitNegotiation::BelowPreferredFloor { supported } = negotiation {
            eprintln!(
                "usdview: adapter max_storage_buffers_per_shader_stage={supported} (< 16 floor); clamped to adapter limit"
            );
        }
    }

    fn new(ctx: CreationContext<'_>) -> Self {
        let config = PENDING_MARA_LAUNCH
            .lock()
            .expect("mara launch mutex poisoned")
            .take()
            .unwrap_or_else(|| MaraLaunchConfig {
                launch_options: LaunchOptions::default(),
                asset_path: "animated_spinner.usda".to_owned(),
                asset_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"),
                is_s8: false,
            });

        let plugin_asset_root = config.asset_root.clone();
        let launch_options = config.launch_options.clone();
        let asset_path = config.asset_path.clone();
        let asset_root = config.asset_root.clone();
        let is_s8 = config.is_s8;

        let bevy_view = mara_bevy::MaraBevyViewport::with_render_state_plugins_and_content(
            ctx.gpu(),
            move |plugins| {
                plugins.set(bevy::asset::AssetPlugin {
                    file_path: plugin_asset_root.to_string_lossy().into_owned(),
                    ..Default::default()
                })
            },
            move |app: &mut App| {
                app.init_resource::<LoaderLog>()
                    .init_resource::<HostUiInputCapture>();
                composition::configure_shared_application(
                    app,
                    &launch_options,
                    &asset_path,
                    asset_root.clone(),
                    is_s8,
                );
                app.add_systems(
                    Startup,
                    bind_cameras_to_mara_render_target
                        .after(mara_bevy::BevyViewportSet::SetupTarget)
                        .after(spawn_camera_and_ground),
                )
                .add_systems(Update, bind_cameras_to_mara_render_target);
            },
        );

        Self {
            bevy_view,
            workspace: WorkspaceStack::new("usdview-workspace"),
            ui_state: MaraHostUiState::default(),
            shell_initialized: false,
        }
    }

    fn configure_shell(&mut self, bar: &mut ShellBar) {
        bar.views = vec![ShellView::new("viewport", "cube", "USD Viewport")];
        bar.active = Some("viewport");
    }

    fn on_shell_event(&mut self, _event: ShellEvent, _ctx: &mut MaraHostCtx<'_>) {}

    fn update(&mut self, host: &mut MaraHostCtx<'_>) {
        mara_core::style::set_theme(mara_core::style::theme_pro(Mode::Dark));
        host.apply_theme(AccentColor::default(), GlassOpacity::default());
        let accent = active_accent();

        if !self.shell_initialized {
            host.set_rail_pane_open(panels::RIBBON_LEFT, panels::PANE_TREE, true);
            self.shell_initialized = true;
        }

        self.poll_file_dialogs();

        // Viewport background surface behind Mara's ribbon-avoiding panes.
        let viewport_rect = {
            let mut vctx = host.view_ctx(&mut self.workspace, accent, RibbonAvoidance::all());
            let rect: egui::Rect = vctx.screen_rect().into();
            input::sync_viewport_input(
                host.__internal_egui(),
                rect,
                self.bevy_view.world_mut(),
                &mut self.ui_state,
            );
            self.bevy_view.show(&mut vctx, host.gpu(), accent);
            rect
        };
        let _ = viewport_rect;

        input::handle_shortcuts(host, self.bevy_view.world_mut());

        if let Some(world) = self.bevy_view.world_mut() {
            panels::show_panels(host, world, &mut self.ui_state, accent);

            if world
                .get_resource::<Messages<bevy::app::AppExit>>()
                .is_some_and(|m| !m.is_empty())
            {
                host.request_close();
            }

            let continuous = {
                let rm = world
                    .get_resource::<crate::viewport::api::ViewportReadModelState>()
                    .and_then(|r| r.snapshot());
                let playing = rm.is_some_and(|s| s.timeline.playing || s.physics_running);
                let flying = world
                    .get_resource::<FlyTo>()
                    .is_some_and(|f| f.remaining > 0.0);
                let mounted = world
                    .get_resource::<CameraMount>()
                    .is_some_and(|m| matches!(m, CameraMount::Mounted { .. }));
                let benchmarking = world
                    .contains_resource::<crate::viewport::diagnostics::performance::BenchmarkLaunchConfig>(
                    );
                playing || flying || mounted || benchmarking
            };
            self.bevy_view.set_continuous_rendering(continuous);
        } else {
            let rail = RibbonRail::view_left(panels::RIBBON_LEFT, "usdview.mara.ribbons");
            let _ = host.show_ribbon_rail(rail, accent);
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/unit/viewport/app/mara_host_tests.rs"]
mod tests;
