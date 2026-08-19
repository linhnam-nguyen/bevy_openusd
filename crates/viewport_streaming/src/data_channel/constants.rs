pub const CONTROL_CHANNEL_LABEL: &str = "viewport-control";
pub const INPUT_CHANNEL_LABEL: &str = "viewport-input";
pub const CONTROL_CHANNEL_PROTOCOL: &str = "usd-hub.viewport.v1";
pub const INPUT_CHANNEL_PROTOCOL: &str = "usd-hub.viewport-input.v1";

// Browser DataChannels commonly reject application messages around 16 KiB.
// Keep a safety margin for the JSON envelope and browser/runtime variation.
pub(super) const MAX_APPLICATION_MESSAGE_BYTES: usize = 12 * 1024;
pub(super) const INITIAL_SNAPSHOT_CHUNK_PRIMS: usize = 128;
pub(super) const INITIAL_RUNTIME_MANIFEST_CHUNK_REFS: usize = 64;
pub(super) const INITIAL_RUNTIME_BLOB_CHUNK_BYTES: usize = 2048;
pub(super) const MAX_COMPACT_STAGE_DISPLAY_NAME_CHARS: usize = 256;
/// Flow-control notification threshold for the active reliable control channel.
pub(super) const CONTROL_CHANNEL_LOW_WATER_MARK_BYTES: u64 = 64 * 1024;
pub(super) const MAX_RECENT_REQUEST_IDS: usize = 256;
