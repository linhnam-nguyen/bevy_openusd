use super::*;

#[test]
fn wrapper_preserves_authored_playback_domain() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("animated.usda");
    std::fs::write(
        &source_path,
        "#usda 1.0\n( startTimeCode = 1 endTimeCode = 48 timeCodesPerSecond = 24 )\ndef Xform \"Root\" {}\n",
    )?;
    let wrapper = Stage::builder().in_memory("wrapper.usda")?;

    copy_source_time_metadata(&source_path, &wrapper)?;

    assert!(wrapper.has_authored_time_code_range());
    assert_eq!(wrapper.start_time_code(), 1.0);
    assert_eq!(wrapper.end_time_code(), 48.0);
    assert_eq!(wrapper.time_codes_per_second(), 24.0);
    Ok(())
}

#[test]
fn wrapper_preserves_authored_frames_per_second_and_non_default_rate() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("animated_60.usda");
    std::fs::write(
        &source_path,
        "#usda 1.0\n( startTimeCode = 0 endTimeCode = 300 timeCodesPerSecond = 60 framesPerSecond = 30 )\ndef Xform \"Root\" {}\n",
    )?;
    let wrapper = Stage::builder().in_memory("wrapper_60.usda")?;

    copy_source_time_metadata(&source_path, &wrapper)?;

    assert!(wrapper.has_authored_time_code_range());
    assert_eq!(wrapper.start_time_code(), 0.0);
    assert_eq!(wrapper.end_time_code(), 300.0);
    assert_eq!(wrapper.time_codes_per_second(), 60.0);
    assert!(
        wrapper
            .has_authored_stage_metadata(sdf::FieldKey::FramesPerSecond.as_str())
            .unwrap_or(false)
    );
    assert_eq!(wrapper.frames_per_second(), 30.0);
    Ok(())
}

#[test]
fn wrapper_infers_conservative_range_from_time_samples_without_authored_range() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("samples_only.usda");
    std::fs::write(
        &source_path,
        r#"#usda 1.0
(
    defaultPrim = "Root"
    timeCodesPerSecond = 30
)
def Xform "Root" {
    double3 xformOp:translate.timeSamples = {
        15: (0, 0, 0),
        105: (10, 0, 0),
    }
    uniform token[] xformOpOrder = ["xformOp:translate"]
}
"#,
    )?;
    let wrapper = Stage::builder().in_memory("wrapper_inferred.usda")?;

    copy_source_time_metadata(&source_path, &wrapper)?;

    assert!(wrapper.has_authored_time_code_range());
    assert_eq!(wrapper.start_time_code(), 15.0);
    assert_eq!(wrapper.end_time_code(), 105.0);
    assert_eq!(wrapper.time_codes_per_second(), 30.0);
    Ok(())
}

#[test]
fn wrapper_does_not_fabricate_range_for_static_or_single_sample_source() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let static_path = directory.path().join("static.usda");
    std::fs::write(
        &static_path,
        "#usda 1.0\n( defaultPrim = \"Root\" )\ndef Xform \"Root\" { double3 xformOp:translate = (1, 2, 3) }\n",
    )?;
    let wrapper = Stage::builder().in_memory("wrapper_static.usda")?;
    copy_source_time_metadata(&static_path, &wrapper)?;
    assert!(!wrapper.has_authored_time_code_range());

    let single_sample_path = directory.path().join("single_sample.usda");
    std::fs::write(
        &single_sample_path,
        "#usda 1.0\n( defaultPrim = \"Root\" )\ndef Xform \"Root\" { double3 xformOp:translate.timeSamples = { 5: (1, 2, 3) } }\n",
    )?;
    let wrapper_single = Stage::builder().in_memory("wrapper_single.usda")?;
    copy_source_time_metadata(&single_sample_path, &wrapper_single)?;
    assert!(!wrapper_single.has_authored_time_code_range());
    Ok(())
}
