//! [`UsdSnippet`] (PLAN P3) — the runtime value the [`usd!`](macro@crate::usd)
//! macro expands to: a validated `usda` fragment that can be opened as a stage
//! and projected through the routing registry (dogfooding P1).
//!
//! In-place composition of a snippet into an already-live stage awaits an
//! openusd in-memory `Layer::from_string` (there is no public text→`Layer`
//! constructor yet); until then [`UsdSnippet::open_stage`] materializes the
//! snippet through a temp file. Tracked as a PLAN P3 / upstreaming follow-up.

use std::sync::atomic::{AtomicU64, Ordering};

use openusd::sdf;
use openusd::usd::Stage;

/// A `usda` text fragment produced by the [`usd!`](macro@crate::usd) macro (or
/// built directly). The text is already compile-time validated when it comes
/// from the macro; [`parse`](UsdSnippet::parse) re-checks an ad-hoc one.
#[derive(Debug, Clone)]
pub struct UsdSnippet {
    text: String,
}

impl UsdSnippet {
    /// Wrap raw `usda` text. The macro calls this with validated text; callers
    /// building text by hand should [`parse`](UsdSnippet::parse) to check it.
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    /// The `usda` source.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Parse the snippet into an in-memory [`sdf::Data`] (validation only; does
    /// not compose a stage).
    pub fn parse(&self) -> anyhow::Result<sdf::Data> {
        openusd::usda::parse(&self.text)
    }

    /// Open the snippet as a standalone [`Stage`]. Wrap the result in
    /// `LiveStage` to project it through the routing registry.
    ///
    /// Currently materializes through a temporary `.usda` file, because openusd
    /// has no public in-memory text→`Layer`/`Stage` constructor yet. The file
    /// is written under the OS temp dir with a process-unique name and removed
    /// after the stage is opened (the stage holds the parsed data, not the
    /// file).
    pub fn open_stage(&self) -> anyhow::Result<Stage> {
        use std::io::Write;

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let file_name = format!("usd_snippet_{}_{}.usda", std::process::id(), n);
        let path = std::env::temp_dir().join(file_name);

        {
            let mut file = std::fs::File::create(&path)?;
            file.write_all(self.text.as_bytes())?;
        }
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("temp path is not valid UTF-8"))?;
        let stage = Stage::open(path_str);
        // Best-effort cleanup; the opened stage no longer needs the file.
        let _ = std::fs::remove_file(&path);
        stage
    }
}

#[cfg(test)]
#[path = "../tests/unit/snippet_tests.rs"]
mod tests;
