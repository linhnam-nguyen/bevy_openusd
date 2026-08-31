use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use openusd::sdf::Value;

use super::regular_file;

pub(super) fn resolve_udim_pattern(layer_path: &Path, authored: &str) -> Result<Vec<PathBuf>> {
    let authored_path = Path::new(authored);
    ensure!(
        !authored_path.is_absolute() || authored_path.file_name().is_some(),
        "UDIM asset path must have a filename: {authored}"
    );
    let parent = if authored_path.is_absolute() {
        authored_path
            .parent()
            .context("UDIM asset path has no parent directory")?
            .to_owned()
    } else {
        layer_path
            .parent()
            .context("USD dependency layer has no parent directory")?
            .join(
                authored_path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("")),
            )
    };
    let file_pattern = authored_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("UDIM asset filename must be valid UTF-8")?;
    let (prefix, suffix) = file_pattern
        .split_once("<UDIM>")
        .context("UDIM asset path has no <UDIM> token")?;
    let mut matches = Vec::new();
    for entry in fs::read_dir(&parent)
        .with_context(|| format!("read UDIM asset directory {}", parent.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(tile) = name
            .strip_prefix(prefix)
            .and_then(|name| name.strip_suffix(suffix))
        else {
            continue;
        };
        if tile.len() != 4
            || !tile.bytes().all(|byte| byte.is_ascii_digit())
            || !(1001..=1999).contains(&tile.parse::<u32>().unwrap_or_default())
        {
            continue;
        }
        matches.push(regular_file(&entry.path())?);
    }
    matches.sort();
    matches.dedup();
    ensure!(
        !matches.is_empty(),
        "UDIM asset pattern has no matching tiles: {authored}"
    );
    Ok(matches)
}

/// Match OpenUSD's template value-clip expansion for dependency discovery.
pub(crate) fn expand_template_asset_paths(
    values: &HashMap<String, Value>,
    template: &str,
) -> Result<Vec<String>> {
    let start = template_time(values, "templateStartTime")?;
    let end = template_time(values, "templateEndTime")?;
    let stride = template_time(values, "templateStride")?;
    ensure!(
        stride.is_finite() && stride > 0.0,
        "template stride must be positive"
    );
    ensure!(
        end.is_finite() && start.is_finite() && end >= start,
        "template time range is invalid"
    );
    let (prefix, width, suffix) = template_hash_pattern(template)?;
    let mut paths = Vec::new();
    let mut time = start;
    while time <= end + 0.0000001 {
        paths.push(format!(
            "{prefix}{:0width$}{suffix}",
            time as i64,
            width = width
        ));
        time += stride;
        ensure!(
            paths.len() <= 1_000_000,
            "template clip expansion is too large"
        );
    }
    ensure!(!paths.is_empty(), "template clip expansion is empty");
    Ok(paths)
}

fn template_time(values: &HashMap<String, Value>, key: &str) -> Result<f64> {
    match values.get(key) {
        Some(Value::Double(value)) => Ok(*value),
        _ => bail!("template clip metadata {key} must be double"),
    }
}

fn template_hash_pattern(template: &str) -> Result<(&str, usize, &str)> {
    let first = template
        .find('#')
        .context("template clip path has no hash pattern")?;
    let prefix = &template[..first];
    let rest = &template[first..];
    let width = rest
        .chars()
        .take_while(|character| *character == '#')
        .count();
    let suffix = &rest[width..];
    ensure!(
        width > 0 && !suffix.contains('#'),
        "template clip hash pattern is invalid"
    );
    Ok((prefix, width, suffix))
}
