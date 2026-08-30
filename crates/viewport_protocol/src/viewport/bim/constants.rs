//! Bounds shared by the typed BIM query/read-model contract.

pub const MAX_BIM_FIELD_KEY_BYTES: usize = 256;
pub const MAX_BIM_CLASSIFICATION_LEVELS: usize = 16;
pub const MAX_BIM_CLASSIFICATION_PAGE_SIZE: u32 = 1_000;
pub const MAX_BIM_REGEX_BYTES: usize = 512;
pub const MAX_BIM_REPLACEMENT_BYTES: usize = 1_024;
pub const MAX_BIM_SEARCH_PAGE_SIZE: u32 = 1_000;
pub const MAX_BIM_SEARCH_OFFSET: u32 = 10_000;
pub const MAX_BIM_SEARCH_GROUPS: usize = 65_536;
pub const MAX_BIM_SELECTION_TARGETS: usize = 256;
pub const MAX_BIM_BATCH_EDITS: usize = 256;
/// Upper bound for one complete property result before transport paging.
pub const MAX_BIM_PROPERTY_COUNT: usize = 65_536;
/// Upper bound for a one-request property page assembly on the client.
pub const MAX_BIM_PROPERTY_PAGES: u32 = 4_096;
pub const UNCLASSIFIED_LABEL: &str = "<Unclassified>";
