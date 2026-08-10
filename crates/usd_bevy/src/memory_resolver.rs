//! In-memory USDZ support for the WebAssembly asset-loader path.
//!
//! The browser cannot create the temporary files used by the native loader.
//! This module keeps either every entry from a self-contained USDZ package or
//! one standalone USD layer in memory and exposes it through OpenUSD's
//! `ar::Resolver` seam.  The synthetic paths are *identifiers*, not paths on
//! the browser's filesystem: `open_asset` always returns a `Cursor<Vec<u8>>`
//! from the package map.

use std::collections::HashMap;
use std::io::{self, Read};

use openusd::ar::{Asset, ResolvedPath, Resolver};

/// A parsed self-contained USD input ready for a custom resolver.
///
/// `embedded` intentionally excludes USD layers.  It is passed to
/// `build::stage_to_scene` so the material path can decode textures directly
/// from the archive without asking the browser for a filesystem path.
#[cfg_attr(test, allow(dead_code))]
pub(crate) struct InMemoryUsdPackage {
    pub(crate) root_identifier: String,
    pub(crate) resolver: InMemoryResolver,
    pub(crate) embedded: HashMap<String, Vec<u8>>,
    /// Plain-text layers are retained separately for the existing SkelAnimation
    /// sidecar scanner.  The stage itself still reads them through `resolver`.
    pub(crate) text_layers: Vec<(String, String)>,
}

impl InMemoryUsdPackage {
    /// Unpack a USDZ archive without materialising any entry on disk.
    pub(crate) fn from_usdz(bytes: &[u8], identity_hint: &str) -> Result<Self, String> {
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|error| error.to_string())?;

        let mut root_name = None;
        for index in 0..archive.len() {
            let entry = archive.by_index(index).map_err(|error| error.to_string())?;
            if is_usd_layer_name(entry.name()) {
                root_name = Some(entry.name().to_owned());
                break;
            }
        }
        let root_name = root_name.ok_or_else(|| "USDZ archive contains no USD layer".to_owned())?;

        // The package bytes are part of both the synthetic root and resolver
        // identity.  Two cached assets with distinct package bytes must not
        // compare as the same layer stack merely because their Bevy paths match.
        let fingerprint = stable_fingerprint(bytes);
        let virtual_root = format!("/__bevy_openusd_memory__/usdz-{fingerprint:016x}");
        let mut assets = HashMap::with_capacity(archive.len());
        let mut embedded = HashMap::new();
        let mut text_layers = Vec::new();

        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
            if entry.is_dir() {
                continue;
            }

            let name = normalize_archive_name(entry.name())?;
            let mut entry_bytes = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut entry_bytes)
                .map_err(|error| error.to_string())?;

            let identifier = format!("{virtual_root}/{name}");
            if assets.insert(identifier, entry_bytes.clone()).is_some() {
                return Err(format!(
                    "USDZ archive contains duplicate normalized entry {name:?}"
                ));
            }

            if is_usd_layer_name(&name) {
                if is_text_usd_name_and_bytes(&name, &entry_bytes)
                    && let Ok(text) = std::str::from_utf8(&entry_bytes)
                {
                    text_layers.push((name, text.to_owned()));
                }
            } else {
                // Preserve the archive-relative spelling.  `texture::lookup_embedded`
                // already normalizes common `./textures/foo.png` variants.
                embedded.insert(name, entry_bytes);
            }
        }

        let root_name = normalize_archive_name(&root_name)?;
        let root_identifier = format!("{virtual_root}/{root_name}");
        if !assets.contains_key(&root_identifier) {
            return Err(format!(
                "root layer {root_name:?} is missing from USDZ archive"
            ));
        }

        Ok(Self {
            root_identifier,
            resolver: InMemoryResolver {
                assets,
                virtual_root,
                identity: format!("in-memory-usdz:{identity_hint}:{fingerprint:016x}"),
            },
            embedded,
            text_layers,
        })
    }

    /// Expose one loose, self-contained root layer through the same resolver.
    ///
    /// This is intentionally only a convenience for small bundled `.usda`,
    /// `.usd`, or `.usdc` files.  There are no sibling bytes to resolve here;
    /// stages with external references, payloads, or textures must be packed
    /// as a USDZ instead.
    pub(crate) fn from_single_layer(bytes: &[u8], identity_hint: &str) -> Result<Self, String> {
        let root_name = normalize_archive_name(identity_hint)?;
        if !is_usd_layer_name(&root_name) {
            return Err(format!(
                "in-memory root has no supported USD extension: {root_name:?}"
            ));
        }

        let fingerprint = stable_fingerprint(bytes);
        let virtual_root = format!("/__bevy_openusd_memory__/root-{fingerprint:016x}");
        let root_identifier = format!("{virtual_root}/{root_name}");
        let mut assets = HashMap::with_capacity(1);
        assets.insert(root_identifier.clone(), bytes.to_vec());

        let text_layers = is_text_usd_name_and_bytes(&root_name, bytes)
            .then(|| {
                std::str::from_utf8(bytes)
                    .ok()
                    .map(|text| (root_name.clone(), text.to_owned()))
            })
            .flatten()
            .into_iter()
            .collect();

        Ok(Self {
            root_identifier,
            resolver: InMemoryResolver {
                assets,
                virtual_root,
                identity: format!("in-memory-usd:{identity_hint}:{fingerprint:016x}"),
            },
            embedded: HashMap::new(),
            text_layers,
        })
    }
}

/// Resolver over one in-memory USDZ package.
///
/// The resolver deliberately never delegates to `DefaultResolver`: falling
/// through to it would make a browser load CWD/filesystem-dependent again.
pub(crate) struct InMemoryResolver {
    assets: HashMap<String, Vec<u8>>,
    virtual_root: String,
    identity: String,
}

#[cfg_attr(test, allow(dead_code))]
impl InMemoryResolver {
    /// Add an ephemeral USDA layer, used for in-memory variant overrides before
    /// handing this resolver to `StageBuilder`.
    pub(crate) fn insert_session_layer(&mut self, text: String) -> String {
        let fingerprint = stable_fingerprint(text.as_bytes());
        let identifier = format!(
            "{}/__bevy_openusd_session_{fingerprint:016x}.usda",
            self.virtual_root
        );
        self.assets.insert(identifier.clone(), text.into_bytes());
        identifier
    }

    fn identifier_for(&self, asset_path: &str, anchor: Option<&ResolvedPath>) -> Option<String> {
        if asset_path.is_empty() {
            return None;
        }

        let asset_path = asset_path.replace('\\', "/");
        let candidate = if asset_path.starts_with('/') {
            asset_path
        } else if let Some(anchor) = anchor {
            let anchor = anchor.to_string_lossy().replace('\\', "/");
            if !anchor.starts_with(&self.virtual_root) {
                return None;
            }
            let parent = anchor
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or(&anchor);
            format!("{parent}/{asset_path}")
        } else {
            format!("{}/{asset_path}", self.virtual_root)
        };

        let normalized = normalize_virtual_path(&candidate)?;
        let in_package = normalized == self.virtual_root
            || normalized
                .strip_prefix(&self.virtual_root)
                .is_some_and(|suffix| suffix.starts_with('/'));
        in_package.then_some(normalized)
    }
}

impl Resolver for InMemoryResolver {
    fn create_identifier(&self, asset_path: &str, anchor: Option<&ResolvedPath>) -> String {
        self.identifier_for(asset_path, anchor)
            // Preserve a stable unresolved spelling so OpenUSD can report the
            // authored asset path instead of consulting the host filesystem.
            .unwrap_or_else(|| asset_path.replace('\\', "/"))
    }

    fn resolve(&self, asset_path: &str) -> Option<ResolvedPath> {
        let identifier = self.identifier_for(asset_path, None)?;
        self.assets
            .contains_key(&identifier)
            .then(|| ResolvedPath::new(identifier))
    }

    fn resolve_for_new_asset(&self, asset_path: &str) -> Option<ResolvedPath> {
        self.identifier_for(asset_path, None).map(ResolvedPath::new)
    }

    fn open_asset(&self, resolved_path: &ResolvedPath) -> io::Result<Box<dyn Asset>> {
        let identifier =
            normalize_virtual_path(&resolved_path.to_string_lossy()).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid in-memory USDZ path: {resolved_path}"),
                )
            })?;
        let bytes = self.assets.get(&identifier).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("in-memory USDZ entry not found: {resolved_path}"),
            )
        })?;

        // Match the native StripMetadataResolver for every text layer, not just
        // the root.  This keeps Omniverse-only metadata from being a browser-only
        // parser regression when a sublayer or reference contains it.
        let bytes = if is_text_usd_name_and_bytes(&identifier, bytes) {
            usd_schema::third_party::strip_metadata::strip_unsupported_prim_metadata(bytes)
        } else {
            bytes.clone()
        };
        Ok(Box::new(io::Cursor::new(bytes)))
    }

    fn identity(&self) -> String {
        self.identity.clone()
    }

    fn get_modification_timestamp(
        &self,
        _asset_path: &str,
        _resolved_path: &ResolvedPath,
    ) -> Option<std::time::SystemTime> {
        // Archive bytes are immutable for the lifetime of this resolver.  More
        // importantly, do not ask the browser for a filesystem timestamp.
        None
    }
}

fn is_usd_layer_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".usda") || lower.ends_with(".usdc") || lower.ends_with(".usd")
}

fn is_text_usd_name_and_bytes(name: &str, bytes: &[u8]) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".usda") || (lower.ends_with(".usd") && is_text_usd(bytes))
}

fn is_text_usd(bytes: &[u8]) -> bool {
    let start = bytes
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 0xEF | 0xBB | 0xBF))
        .unwrap_or(bytes.len());
    bytes[start..].starts_with(b"#usda")
}

/// Normalize a ZIP entry name without letting it leave the package root.
fn normalize_archive_name(name: &str) -> Result<String, String> {
    let name = name.replace('\\', "/");
    let mut segments = Vec::new();
    for segment in name.split('/') {
        match segment {
            "" | "." => continue,
            ".." => {
                return Err(format!("USDZ archive entry escapes package root: {name:?}"));
            }
            _ => segments.push(segment),
        }
    }
    if segments.is_empty() {
        return Err(format!("USDZ archive entry has no file name: {name:?}"));
    }
    Ok(segments.join("/"))
}

/// Lexically normalize a synthetic absolute path without touching the host
/// filesystem.  Returning `None` for an attempted escape prevents an authored
/// `../../outside.usda` arc from resolving outside the archive map.
fn normalize_virtual_path(path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => continue,
            ".." => {
                segments.pop()?;
            }
            _ => segments.push(segment),
        }
    }
    Some(format!("/{}", segments.join("/")))
}

/// FNV-1a gives the synthetic root/identity a deterministic content marker
/// without pulling a hashing dependency into the browser bundle.
fn stable_fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use openusd::ar::{ResolvedPath, Resolver};
    use openusd::sdf::Value;
    use openusd::usd::{Stage, TimeCode};
    use zip::write::SimpleFileOptions;

    use super::InMemoryUsdPackage;

    fn package(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .expect("start archive entry");
            writer.write_all(bytes).expect("write archive entry");
        }
        writer.finish().expect("finish archive").into_inner()
    }

    #[test]
    fn resolves_internal_layers_and_retains_embedded_media() {
        let bytes = package(&[
            (
                "root.usda",
                b"#usda 1.0\n(defaultPrim = \"World\")\ndef \"World\" (prepend references = @./layers/inner.usda@</Inner>) {}\n",
            ),
            (
                "layers/inner.usda",
                b"#usda 1.0\ndef \"Inner\" { custom int probe = 42 }\n",
            ),
            ("textures/albedo.png", b"not-a-real-png"),
        ]);

        let package = InMemoryUsdPackage::from_usdz(&bytes, "fixture.usdz").expect("unpack USDZ");
        assert_eq!(
            package.embedded.get("textures/albedo.png"),
            Some(&b"not-a-real-png".to_vec())
        );

        let root = package.root_identifier.clone();
        let stage = Stage::builder()
            .resolver(package.resolver)
            .open(&root)
            .expect("compose in-memory package");
        assert_eq!(
            stage
                .attribute("/World.probe")
                .get_at::<Value>(TimeCode::new(0.0))
                .expect("read composed value"),
            Some(Value::Int(42))
        );
    }

    #[test]
    fn resolver_does_not_escape_the_virtual_package_root() {
        let bytes = package(&[("root.usda", b"#usda 1.0\ndef \"World\" {}\n")]);
        let package = InMemoryUsdPackage::from_usdz(&bytes, "fixture.usdz").expect("unpack USDZ");
        let root = ResolvedPath::new(package.root_identifier.clone());

        let escaped = package
            .resolver
            .create_identifier("../../outside.usda", Some(&root));
        assert!(package.resolver.resolve(&escaped).is_none());
    }

    #[test]
    fn resolves_a_single_plain_usda_root_without_filesystem_access() {
        let package = InMemoryUsdPackage::from_single_layer(
            b"#usda 1.0\n(defaultPrim = \"World\")\ndef \"World\" { custom int probe = 7 }\n",
            "assets/animated_spinner.usda",
        )
        .expect("make single-layer package");
        let root = package.root_identifier.clone();
        let stage = Stage::builder()
            .resolver(package.resolver)
            .open(&root)
            .expect("open in-memory USDA");
        assert_eq!(
            stage
                .attribute("/World.probe")
                .get_at::<Value>(TimeCode::new(0.0))
                .expect("read root value"),
            Some(Value::Int(7))
        );
    }
}
