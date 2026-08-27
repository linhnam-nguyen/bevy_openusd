//! Pure semantic and diff domain types shared by USDHub subsystems.
//!
//! This crate deliberately has no dependency on Bevy, OpenUSD, Turso, gix, or
//! any transport implementation. It contains stable values that can be
//! produced from a live stage and compared across revisions.

pub mod change;
pub mod hash;
pub mod identity;
pub mod measurement;
pub mod semantic;
pub mod signature;
pub mod snapshot;
pub mod value;

pub use change::{ChangeFlags, PresenceState};
pub use hash::{HashDigest, HashDigestError};
pub use identity::{EntityKey, IdentitySource};
pub use measurement::{MeasurementMetadata, QuantitySpecId, UnitId};
pub use semantic::SemanticInfo;
pub use signature::{BlobId, Bounds3, GeometrySignature, QuantizedPoint3, TransformSignature};
pub use snapshot::{
    EntitySnapshot, SemanticProperty, SemanticSnapshot, SnapshotId, SnapshotSource,
};
pub use value::CanonicalValue;
