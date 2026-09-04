//! Minimal library-facing Project module.
//!
//! The viewer binary has additional render and recovery modules under
//! src/project.rs. The native host only links this smaller application
//! boundary so read commands do not pull the viewer composition root.

#[path = "project/blob_store.rs"]
pub(crate) mod blob_store;
#[path = "project/commit/mod.rs"]
pub(crate) mod commit;

#[path = "project_api_catalog.rs"]
pub(crate) mod catalog;

#[path = "project_api_scene.rs"]
pub(crate) mod scene;

#[path = "project/cache.rs"]
pub(crate) mod cache;
#[path = "project/cache_hydration.rs"]
pub(crate) mod cache_hydration;
#[path = "project/cache_warm_runtime.rs"]
pub(crate) mod cache_warm_runtime;
#[path = "project/cache_warmer.rs"]
pub(crate) mod cache_warmer;
#[path = "project/ghost_cache/mod.rs"]
pub(crate) mod ghost_cache;
#[path = "project/link.rs"]
pub(crate) mod link;
#[path = "project/model_import.rs"]
pub(crate) mod model_import;
#[path = "project/model_wrapper.rs"]
pub(crate) mod model_wrapper;
#[path = "project/recovery.rs"]
pub(crate) mod recovery;
#[path = "project/recovery_worker.rs"]
pub(crate) mod recovery_worker;
#[path = "project/runtime_delivery.rs"]
pub(crate) mod runtime_delivery;
#[path = "project/runtime_payload.rs"]
pub(crate) mod runtime_payload;
#[path = "project/semantic_store/mod.rs"]
pub(crate) mod semantic_store;
#[path = "project/source_closure.rs"]
pub(crate) mod source_closure;
#[path = "project/spatial.rs"]
pub(crate) mod spatial;

#[path = "project/storage.rs"]
pub(crate) mod storage;

#[path = "project/service/mod.rs"]
pub mod service;
