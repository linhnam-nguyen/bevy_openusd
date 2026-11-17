//! Feature-oriented Project architecture and domain subsystems.
//!
//! The `project` module is organized into 13 feature submodules, with `service`
//! providing the public application facade and the remaining modules owning
//! their domain logic, application operations, and internal dependencies:
//! - `assets`: dependency discovery, resolution, localization, and spatial conventions
//! - `cache`: persistent derived data, hydration, and background warming
//! - `catalog`: workspace registry, manifests, registration, and project discovery
//! - `exchange`: export, portability, and operation progress tracking
//! - `lifecycle`: project creation, deletion, and renaming
//! - `models`: imported models, wrappers, and source links
//! - `runtime`: communication with the runtime stage owner and payload delivery
//! - `scene`: managed scene composition, membership, and adoption
//! - `semantic_store`: persistent BIM semantic snapshots, queries, Turso, and synchronization
//! - `service`: public Project API and application boundary
//! - `stages`: live stage activation, editing, metadata, inspection, recovery, and archival
//! - `storage`: project filesystem layout, migration, and durability
//! - `versioning`: Git branches, commits, and commit coordination

pub(crate) mod assets;
pub(crate) mod cache;
pub(crate) mod catalog;
pub(crate) mod exchange;
pub(crate) mod lifecycle;
pub(crate) mod models;
pub(crate) mod runtime;
pub(crate) mod scene;
pub(crate) mod semantic_store;
pub mod service;
pub(crate) mod stages;
pub(crate) mod storage;
pub(crate) mod versioning;
