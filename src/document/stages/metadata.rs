//! Stage-level metadata copied onto USDHub-owned composition wrappers.

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result, ensure};
use openusd::{
    ar::DefaultResolver,
    sdf,
    usd::{InitialLoadSet, Stage},
};

/// Effective playback range expressed in a stage's own time-code units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TimeCodeRange {
    pub start: f64,
    pub end: f64,
}

impl TimeCodeRange {
    pub(crate) fn new(start: f64, end: f64) -> Result<Self> {
        ensure!(
            start.is_finite() && end.is_finite(),
            "playback time code range must be finite"
        );
        ensure!(
            end >= start,
            "playback endTimeCode ({end}) must be greater than or equal to startTimeCode ({start})"
        );
        Ok(Self { start, end })
    }

    /// Convert a validated range to elapsed seconds without accepting an
    /// underflowed positive interval as a static stage.
    pub(crate) fn duration_seconds(self, rate: f64) -> Result<f64> {
        let range = Self::new(self.start, self.end)?;
        ensure!(rate.is_finite() && rate > 0.0, "invalid playback time rate {rate}");
        let duration = (range.end - range.start) / rate;
        ensure!(duration.is_finite() && (!range.is_non_empty() || duration > 0.0),
            "USD playback duration is not representable");
        Ok(duration)
    }

    pub(crate) fn is_non_empty(self) -> bool {
        self.end > self.start
    }

    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// Inspected stage-level temporal metadata resolved from a USD source or wrapper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StageTimeMetadata {
    pub range: Option<TimeCodeRange>,
    pub time_codes_per_second: f64,
    pub frames_per_second: Option<f64>,
}

/// Read the authored playback domain and timing rates from an open stage.
///
/// When `startTimeCode`/`endTimeCode` are unauthored and `infer_from_samples`
/// is `true`, performs a conservative scan over composed attribute time samples
/// and returns a range only when finite samples span a positive interval.
pub(crate) fn inspect_stage_time_metadata(
    stage: &Stage,
    infer_from_samples: bool,
) -> Result<StageTimeMetadata> {
    let time_codes_per_second = stage.time_codes_per_second();
    ensure!(
        time_codes_per_second.is_finite() && time_codes_per_second > 0.0,
        "USD stage timeCodesPerSecond must be finite and positive (got {time_codes_per_second})"
    );
    let frames_per_second = if stage
        .has_authored_stage_metadata(sdf::FieldKey::FramesPerSecond.as_str())?
    {
        let fps = stage.frames_per_second();
        ensure!(
            fps.is_finite() && fps > 0.0,
            "USD stage framesPerSecond must be finite and positive (got {fps})"
        );
        Some(fps)
    } else {
        None
    };

    let has_start = stage.has_authored_stage_metadata(sdf::FieldKey::StartTimeCode.as_str())?;
    let has_end = stage.has_authored_stage_metadata(sdf::FieldKey::EndTimeCode.as_str())?;
    ensure!(has_start == has_end, "partially authored USD playback range is unsupported");
    let range = if stage.has_authored_time_code_range() {
        Some(TimeCodeRange::new(
            stage.start_time_code(),
            stage.end_time_code(),
        )?)
    } else if infer_from_samples {
        infer_composed_time_sample_range(stage)?
    } else {
        None
    };

    if let Some(range) = range {
        range.duration_seconds(time_codes_per_second)?;
    }
    Ok(StageTimeMetadata {
        range,
        time_codes_per_second,
        frames_per_second,
    })
}

/// Inspect a USD source file for its effective playback domain and timing rates.
///
/// Opens with `LoadNone` first for the fast path when `startTimeCode` and
/// `endTimeCode` are authored. Falls back to `LoadAll` only when no authored
/// range is present so payloaded `timeSamples` can be discovered conservatively.
pub(crate) fn inspect_source_time_metadata(source: &Path) -> Result<StageTimeMetadata> {
    let source_string = source
        .to_str()
        .context("USD source path must be valid UTF-8")?;
    let resolver: Arc<dyn openusd::ar::Resolver> = Arc::new(DefaultResolver::with_search_paths([
        source.parent().unwrap_or(Path::new(".")),
    ]));
    let stage = crate::project::assets::closure::open_stage_with_resolver(
        source,
        Arc::clone(&resolver),
        InitialLoadSet::LoadNone,
    )
    .with_context(|| format!("open source playback metadata {source_string}"))?;

    if stage.has_authored_time_code_range() {
        return inspect_stage_time_metadata(&stage, false);
    }

    let full_stage = crate::project::assets::closure::open_stage_with_resolver(
        source,
        resolver,
        InitialLoadSet::LoadAll,
    )
    .with_context(|| format!("open source playback samples {source_string}"))?;
    inspect_stage_time_metadata(&full_stage, true)
}

/// Apply inspected temporal metadata onto a wrapper stage without flattening
/// or copying source opinions.
pub(crate) fn apply_stage_time_metadata(
    stage: &Stage,
    metadata: StageTimeMetadata,
) -> Result<()> {
    ensure!(
        metadata.time_codes_per_second.is_finite() && metadata.time_codes_per_second > 0.0,
        "wrapper timeCodesPerSecond must be finite and positive"
    );
    if let Some(range) = metadata.range {
        range.duration_seconds(metadata.time_codes_per_second)?;
        stage.set_start_time_code(range.start)?;
        stage.set_end_time_code(range.end)?;
    }
    stage.set_time_codes_per_second(metadata.time_codes_per_second)?;
    if let Some(fps) = metadata.frames_per_second {
        ensure!(
            fps.is_finite() && fps > 0.0,
            "wrapper framesPerSecond must be finite and positive"
        );
        stage.set_frames_per_second(fps)?;
    }
    Ok(())
}

/// Preserve the source playback domain without copying or flattening source
/// opinions. The wrapper owns the effective rate and authored range used by
/// the viewport clock; referenced time samples remain source-owned.
pub(crate) fn copy_source_time_metadata(source: &Path, wrapper: &Stage) -> Result<()> {
    let metadata = inspect_source_time_metadata(source)?;
    apply_stage_time_metadata(wrapper, metadata)
}

/// Conservatively derive a `[min, max]` time-code span from composed attribute
/// time samples (including value clips) when a stage lacks an authored range.
///
/// Returns `None` when no finite samples exist or all samples share one time
/// code so static stages never receive a fabricated playback duration.
pub(crate) fn infer_composed_time_sample_range(stage: &Stage) -> Result<Option<TimeCodeRange>> {
    let mut min_time = f64::INFINITY;
    let mut max_time = f64::NEG_INFINITY;
    let mut prim_paths = Vec::new();
    stage.traverse(openusd::usd::PrimPredicate::default(), |path: &sdf::Path| {
        prim_paths.push(path.clone());
    }).context("traverse composed playback samples")?;
    for prim_path in prim_paths {
        let prim = stage.prim(prim_path.clone())
            .with_context(|| format!("inspect playback prim {prim_path}"))?;
        let attributes = prim.attributes()
            .with_context(|| format!("inspect playback attributes on {prim_path}"))?;
        for attribute in attributes {
            let path = attribute.path().clone();
            let Some(times) = stage.time_sample_times(path.clone())
                .with_context(|| format!("inspect playback samples on {path}"))? else {
                continue;
            };
            for time in times {
                ensure!(time.is_finite(), "non-finite time sample on {path}");
                min_time = min_time.min(time);
                max_time = max_time.max(time);
            }
        }
    }
    if min_time.is_finite() && max_time.is_finite() && max_time > min_time {
        Ok(Some(TimeCodeRange::new(min_time, max_time)?))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/project/stages/metadata_tests.rs"]
mod tests;
