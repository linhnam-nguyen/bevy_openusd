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
