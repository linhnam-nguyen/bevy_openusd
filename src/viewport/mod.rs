//! Generic Bevy viewport with exactly four responsibility groups (PLAN §3.5.3).

#[cfg(test)]
#[path = "../../tests/unit/viewport/test_harness.rs"]
pub(crate) mod test_harness;

pub(crate) mod navigation;
pub(crate) mod presentation;
pub(crate) mod rendering;
pub(crate) mod stage;
