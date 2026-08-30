use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use anyhow::{Context, Result};

pub(super) fn copy_file_synced(source: &Path, destination: &Path) -> Result<()> {
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
