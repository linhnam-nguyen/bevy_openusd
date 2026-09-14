use std::{
    fs,
    path::{Path, PathBuf},
};

use project_protocol::{ProjectActivationCommand, ProjectStageTarget};
use tempfile::tempdir;
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, StorageKey,
};
use viewport_streaming::ProjectActivationRequest;

use crate::{
    project::{
        cache::SceneCacheStore,
        cache_contract::{
            SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
            SceneCacheDescriptorV3, SceneCacheIndex, SceneCacheState, SceneSpatialIndex,
        },
        cache_hydration::default_project_cache_config_hash,
        catalog::{manifest_store::ManifestStore, workspace_registry::WorkspaceRegistry},
        scene::{adoption_authoring, authoring},
    },
    viewport::{
        app::project_activation::ProjectStageActivationRuntime,
        diagnostics::animation_debug::{HUMMINGBIRD_MAX_REPEAT_MAD, HUMMINGBIRD_MIN_MAD},
        transport::frame_signature::FrameLumaSample,
    },
};

use super::render_parity_support::{RenderActivationWorld, install_visible_cache};

#[test]
fn project_fresh_and_cache_first_server_frames_preserve_animation_motion() {
    let fresh = RenderHummingbirdFixture::new(false);
    let fresh_target = fresh.prepare_target();
    let mut fresh_world = RenderActivationWorld::new();
    assert!(fresh_world.admit(&fresh.command, "render-fresh-session"));
    assert!(
        fresh_world
            .apply(
                &fresh.command,
                "render-fresh-session",
                Ok(Some(fresh_target))
            )
            .is_some()
    );
    fresh_world.wait_ready();
    let fresh_samples = fresh_world.sample_triplet();
    assert_motion("P2 Project Fresh", &fresh_samples);

    let cached = RenderHummingbirdFixture::new(true);
    let cached_target = cached.prepare_target();
    let mut cached_world = RenderActivationWorld::new();
    assert!(cached_world.admit(&cached.command, "render-cache-session"));
    assert!(
        cached_world
            .apply(
                &cached.command,
                "render-cache-session",
                Ok(Some(cached_target))
            )
            .is_none()
    );
    assert!(
        cached_world
            .world()
            .contains_resource::<crate::viewport::session::SceneCachePresentation>()
    );
    install_visible_cache(&mut cached_world, cached.scene_id);
    cached_world.wait_for_cache_retirement();
    cached_world.frame_camera();
    let cached_samples = cached_world.sample_triplet();
    assert_motion("P3 Project Cache-First", &cached_samples);
}

fn assert_motion(label: &str, samples: &[FrameLumaSample; 3]) {
    let t0 = &samples[0];
    let t1 = &samples[1];
    let round_trip = &samples[2];
    let motion_mad = t0.mad(t1);
    let repeat_mad = t0.mad(round_trip);
    eprintln!(
        "[b0-m0+3-render] {label} render_t0={:016x} render_t1={:016x} render_round_trip={:016x} mean_luma=({:.3},{:.3},{:.3}) mad={motion_mad:.6} repeat_mad={repeat_mad:.6}",
        t0.hash, t1.hash, round_trip.hash, t0.mean_luma, t1.mean_luma, round_trip.mean_luma
    );
    assert_ne!(t0.hash, t1.hash, "{label} pixels must change");
    assert!(motion_mad >= HUMMINGBIRD_MIN_MAD, "{label} motion MAD");
    assert!(
        repeat_mad <= HUMMINGBIRD_MAX_REPEAT_MAD,
        "{label} repeat MAD"
    );
}

struct RenderHummingbirdFixture {
    _directory: tempfile::TempDir,
    project_id: ProjectId,
    scene_id: SceneId,
    project_root: PathBuf,
    command: ProjectActivationCommand,
}

impl RenderHummingbirdFixture {
    fn new(with_cache: bool) -> Self {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/external/hummingbird.usdz");
        let directory = tempdir().expect("render parity fixture directory");
        let project_root = directory.path().join("project");
        usd_git::Repository::init(&project_root).expect("initialize render parity Project");
        let project_id = ProjectId::new_v4();
        let scene_id = SceneId::new_v4();
        let manifest = ProjectManifestV1::new(
            project_id,
            "Render parity Hummingbird Project",
            ProjectRoot::Empty,
            vec![SceneManifestEntry {
                id: scene_id,
                storage_key: StorageKey::new("hummingbird").expect("storage key"),
                display_name: "Hummingbird".to_owned(),
            }],
            Vec::new(),
        );
        ManifestStore::write_manifest_atomic(&project_root, &manifest).expect("write manifest");
        let scene_path = authoring::scene_path(&project_root, scene_id);
        let package_dir = project_root
            .join("imports/scenes")
            .join(scene_id.to_string());
        fs::create_dir_all(&package_dir).expect("create package directory");
        let package_path = package_dir.join("hummingbird.usdz");
        fs::copy(source, &package_path).expect("copy Hummingbird package");
        let spatial =
            crate::project::spatial::inspect_source(&package_path).expect("inspect source");
        fs::create_dir_all(scene_path.parent().expect("scene directory"))
            .expect("create scene directory");
        adoption_authoring::author_scene_wrapper_to_path(
            &scene_path,
            &project_root,
            &scene_path,
            scene_id,
            &package_path,
            &package_path,
            &["/hummingbird_anim_hover_idle_long".to_owned()],
            "Hummingbird",
            &spatial,
            false,
        )
        .expect("author Scene wrapper");
        if with_cache {
            let config_hash = default_project_cache_config_hash();
            let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, 1, config_hash);
            descriptor.state = SceneCacheState::Partial;
            SceneCacheStore::new(&project_root)
                .publish_generation(
                    &descriptor,
                    &SceneCacheIndex {
                        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
                        scene_id,
                        generation: 1,
                        entries: Vec::new(),
                    },
                    &SceneSpatialIndex {
                        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
                        scene_id,
                        generation: 1,
                        entries: Vec::new(),
                    },
                )
                .expect("publish cache metadata");
        }
        let command = ProjectActivationCommand::new(
            if with_cache {
                "render-cache"
            } else {
                "render-fresh"
            },
            1,
            project_id,
            ProjectStageTarget::Scene(scene_id),
        );
        Self {
            _directory: directory,
            project_id,
            scene_id,
            project_root,
            command,
        }
    }

    fn prepare_target(&self) -> crate::project::service::ProjectStageActivationTarget {
        let registry_path = self._directory.path().join("workspace.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("load registry");
        registry
            .register(self.project_id, &self.project_root, None)
            .expect("register Project");
        let runtime = ProjectStageActivationRuntime::with_registry_path(Some(registry_path));
        let request = ProjectActivationRequest {
            session_id: viewport_protocol::SessionId::new("render-preparation"),
            command: self.command.clone(),
        };
        assert!(runtime.submit(request).is_none());
        runtime
            .wait_for_prepared()
            .expect("preparation result")
            .target
            .expect("preparation succeeds")
            .expect("Scene target exists")
    }
}
