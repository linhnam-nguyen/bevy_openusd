use anyhow::Result;
use openusd::usd::Stage;
use usd_model::{SemanticSnapshot, SnapshotSource};
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::super::{SemanticResponse, SemanticWorkingStore};

pub(super) fn snapshot() -> Result<SemanticSnapshot> {
    let stage = Stage::open("tests/stages/custom_attrs_extensive.usda")?;
    SemanticExtractor::new(SemanticConfig::default()).extract(
        &stage,
        SnapshotSource::Working {
            session: "semantic-worker-test".to_owned(),
            live_revision: 1,
        },
    )
}

pub(super) fn response(store: &SemanticWorkingStore) -> SemanticResponse {
    for _ in 0..200 {
        if let Some(response) = store.drain_responses().into_iter().next() {
            return response;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("semantic worker did not respond")
}
