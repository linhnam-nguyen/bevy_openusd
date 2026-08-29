//! Minimal library-facing Project module.
//!
//! The viewer binary has additional render and recovery modules under
//! src/project.rs. The native host only links this smaller application
//! boundary so read commands do not pull the viewer composition root.

#[path = "project_api_catalog.rs"]
pub(crate) mod catalog;

#[path = "project_api_scene.rs"]
pub(crate) mod scene;

#[path = "project/model_import.rs"]
pub(crate) mod model_import;
#[path = "project/model_wrapper.rs"]
pub(crate) mod model_wrapper;
#[path = "project/spatial.rs"]
pub(crate) mod spatial;

#[path = "project/storage.rs"]
pub(crate) mod storage;

#[path = "project/service/mod.rs"]
pub mod service;
