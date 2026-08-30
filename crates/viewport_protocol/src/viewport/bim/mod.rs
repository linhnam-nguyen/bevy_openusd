//! BIM query intent and read models for the viewport protocol.

mod constants;
mod diff;
mod edit;
mod query;
mod read_model;

pub use constants::{
    MAX_BIM_BATCH_EDITS, MAX_BIM_CLASSIFICATION_LEVELS, MAX_BIM_CLASSIFICATION_PAGE_SIZE,
    MAX_BIM_FIELD_KEY_BYTES, MAX_BIM_PROPERTY_COUNT, MAX_BIM_PROPERTY_PAGES, MAX_BIM_REGEX_BYTES,
    MAX_BIM_REPLACEMENT_BYTES, MAX_BIM_SEARCH_GROUPS, MAX_BIM_SEARCH_OFFSET,
    MAX_BIM_SEARCH_PAGE_SIZE, MAX_BIM_SELECTION_TARGETS, UNCLASSIFIED_LABEL,
};
pub use diff::{BimPropertyDiffReadModel, BimPropertyDiffRow, BimPropertyDiffStatus};
pub use edit::{
    BimPropertyEditOutcome, BimPropertyEditStatus, BimPropertyMutation, validate_bim_mutation_batch,
};
pub use query::{
    BimFieldKey, BimPageRequest, BimSearchQuery, ClassificationLevel, ClassificationRecipe,
};
pub use read_model::{
    BimObjectMatch, BimPropertiesDeliveryError, BimPropertiesDeliveryErrorKind, BimPropertiesPage,
    BimPropertiesReadModel, BimPropertyGroupId, BimPropertyGroupReadModel, BimPropertyNameMatch,
    BimPropertyProvenanceReadModel, BimPropertyProvenanceStatus, BimPropertyReadModel,
    BimPropertyValueMatch, BimReplacementPreviewRow, BimSearchResult, BimUnitOption, CommonValue,
};
