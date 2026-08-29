//! Controlled source closure materialization for Project imports.
//!
//! A canonical wrapper must not retain an absolute reference to a user's
//! source directory. Simple sources are copied as one file. Composed sources
//! are copied with their source directory so relative sublayers, references,
//! payloads, and sidecar assets retain their authored layout.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use openusd::usd::{InitialLoadSet, Stage};

/// Copy one import into a fresh Project-owned directory and return the
/// copied source filename relative to that directory.
pub(crate) fn materialize_source_closure(
    source: &Path,
    destination: &Path,
    copy_dependency_directory: bool,
) -> Result<String> {
    let source = validate_source(source)?;
    ensure!(
        !destination.exists(),
        "source-closure destination already exists"
    );
    let source_parent = source
        .parent()
        .context("USD source has no parent directory")?
        .to_path_buf();
    let layer_paths = if copy_dependency_directory {
        validate_layer_paths(&source, &source_parent)?
    } else {
        Vec::new()
    };

    fs::create_dir_all(destination)
        .with_context(|| format!("create source closure {}", destination.display()))?;
    let source_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .context("USD source filename must be valid UTF-8")?
        .to_owned();

    if copy_dependency_directory && !destination.starts_with(&source_parent) {
        copy_directory_without_symlinks(&source_parent, destination)?;
    } else {
        // A Project-local source can have the destination below its own
        // parent. Copy only the resolved source/layer closure to avoid
        // recursively copying .usdhub into itself.
        copy_file_synced(&source, &destination.join(&source_name))?;
        for layer_path in layer_paths {
            let relative = layer_path
                .strip_prefix(&source_parent)
                .with_context(|| format!("source closure escapes {}", source_parent.display()))?;
            let destination_path = destination.join(relative);
            if destination_path == destination.join(&source_name) {
                continue;
            }
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create source closure {}", parent.display()))?;
            }
            copy_file_synced(&layer_path, &destination_path)?;
        }
    }

    ensure!(
        destination.join(&source_name).is_file(),
        "source closure did not materialize its root source"
    );
    Ok(source_name)
}

fn validate_source(source: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("read USD source metadata {}", source.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "USD import source must be a regular non-symlink file"
    );
    fs::canonicalize(source)
        .with_context(|| format!("canonicalize USD source {}", source.display()))
}

fn validate_layer_paths(source: &Path, source_parent: &Path) -> Result<Vec<PathBuf>> {
    let source_string = source
        .to_str()
        .context("USD source path must be valid UTF-8")?;
    let stage = Stage::builder()
        .load(InitialLoadSet::LoadNone)
        .open(source_string)
        .context("open USD source while preparing its closure")?;
    let mut layer_paths = Vec::new();
    for identifier in stage.layer_stack() {
        if identifier.starts_with("anon:") {
            bail!("USD source closure contains an anonymous layer");
        }
        let identifier_path = Path::new(&identifier);
        let candidate = if identifier_path.is_absolute() {
            identifier_path.to_path_buf()
        } else {
            source_parent.join(identifier_path)
        };
        let metadata = fs::symlink_metadata(&candidate)
            .with_context(|| format!("read USD dependency {}", candidate.display()))?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "USD source closure rejects symlink dependency {}",
            candidate.display()
        );
        let canonical = fs::canonicalize(&candidate)
            .with_context(|| format!("canonicalize USD dependency {}", candidate.display()))?;
        ensure!(
            canonical.starts_with(source_parent),
            "USD source dependency escapes its source directory: {}",
            candidate.display()
        );
        if canonical != source && !layer_paths.iter().any(|path| path == &canonical) {
            layer_paths.push(canonical);
        }
    }
    Ok(layer_paths)
}

fn copy_directory_without_symlinks(source: &Path, destination: &Path) -> Result<()> {
    let mut entries = fs::read_dir(source)
        .with_context(|| format!("read source closure {}", source.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source_path = entry.path();
        let relative = source_path
            .strip_prefix(source)
            .with_context(|| format!("relativize source closure {}", source_path.display()))?;
        let destination_path = destination.join(relative);
        let metadata = fs::symlink_metadata(&source_path)
            .with_context(|| format!("read source closure metadata {}", source_path.display()))?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "source closure rejects symlink {}",
            source_path.display()
        );
        if metadata.is_dir() {
            fs::create_dir_all(&destination_path)
                .with_context(|| format!("create source closure {}", destination_path.display()))?;
            copy_directory_without_symlinks(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            copy_file_synced(&source_path, &destination_path)?;
        } else {
            bail!(
                "source closure contains unsupported entry {}",
                source_path.display()
            );
        }
    }
    Ok(())
}

fn copy_file_synced(source: &Path, destination: &Path) -> Result<()> {
    let mut input = File::open(source)
        .with_context(|| format!("open source closure file {}", source.display()))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .with_context(|| format!("create source closure file {}", destination.display()))?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .context("read source closure file")?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .context("write source closure file")?;
    }
    output.sync_all().context("sync source closure file")?;
    if let Some(parent) = destination.parent() {
        let directory = File::open(parent).context("open source closure directory")?;
        directory
            .sync_all()
            .context("sync source closure directory")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::tempdir;

    use super::*;

    fn write_composed_source(directory: &Path) -> PathBuf {
        let dependency = directory.join("dependency.usda");
        fs::write(
            &dependency,
            "#usda 1.0\n(\n defaultPrim = \"Asset\"\n)\ndef Xform \"Asset\" (kind = \"component\") {}\n",
        )
        .unwrap();
        fs::write(directory.join("texture.bin"), b"texture").unwrap();
        let source = directory.join("assembly.usda");
        fs::write(
            &source,
            "#usda 1.0\n(\n defaultPrim = \"Assembly\"\n)\ndef Xform \"Assembly\" (kind = \"assembly\" references = @./dependency.usda@</Asset>) {}\n",
        )
        .unwrap();
        source
    }

    #[test]
    fn composed_source_closure_survives_source_directory_removal() -> Result<()> {
        let source_directory = tempdir()?;
        let source = write_composed_source(source_directory.path());
        let destination = tempdir()?.path().join("closure");
        let source_name = materialize_source_closure(&source, &destination, true)?;

        assert_eq!(source_name, "assembly.usda");
        assert!(destination.join("dependency.usda").is_file());
        assert_eq!(fs::read(destination.join("texture.bin"))?, b"texture");
        drop(source_directory);
        assert!(Stage::open(&destination.join(source_name).to_string_lossy()).is_ok());
        Ok(())
    }

    #[test]
    fn symlinked_source_is_rejected() -> Result<()> {
        let directory = tempdir()?;
        let actual = directory.path().join("actual.usda");
        fs::write(&actual, "#usda 1.0\n")?;
        let source = directory.path().join("source.usda");
        symlink(&actual, &source)?;
        let destination = tempdir()?.path().join("closure");

        assert!(materialize_source_closure(&source, &destination, false).is_err());
        Ok(())
    }

    #[test]
    fn existing_destination_is_a_collision() -> Result<()> {
        let directory = tempdir()?;
        let source = directory.path().join("source.usda");
        fs::write(&source, "#usda 1.0\n")?;
        let destination = directory.path().join("closure");
        fs::create_dir_all(&destination)?;
        fs::write(destination.join("existing"), b"collision")?;

        assert!(materialize_source_closure(&source, &destination, false).is_err());
        Ok(())
    }
}
