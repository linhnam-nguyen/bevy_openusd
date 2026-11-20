use std::cell::RefCell;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context, Result, ensure};
use openusd::ar::{
    self, Asset, AssetBuffer, DefaultResolver, ResolvedPath, Resolver, SharedBuffer,
};
use openusd::usd::{InitialLoadSet, PrimPredicate, Stage};
use openusd::usd_utils::all_dependencies;

use super::change::LiveRevision;

type LayerBytes = HashMap<PathBuf, SharedBuffer>;

pub(crate) struct SnapshotTextures {
    pub(super) stage_identity: (u64, u64),
    source: Rc<RefCell<LayerBytes>>,
}

impl SnapshotTextures {
    pub(crate) fn read(&self, path: &str) -> Option<Vec<u8>> {
        let resolver = SnapshotResolver {
            filesystem: DefaultResolver::new(),
            source: self.source.clone(),
            revisions: HashMap::new(),
            sealed: true,
        };
        resolver
            .open_asset(&resolver.resolve(path)?)
            .ok()?
            .into_buffer()
            .ok()
            .map(|bytes| bytes.as_ref().to_vec())
    }
}

/// A file-backed document's projection context. OpenUSD owns composition and
/// payload rules in each stage. Only immutable encoded inputs cross registries.
pub(super) struct VisualStage {
    path: PathBuf,
    source: Rc<RefCell<LayerBytes>>,
    revisions: LayerBytes,
    pub(super) stage: Stage,
    pub(super) authoring_revision: LiveRevision,
}

struct SnapshotResolver {
    filesystem: DefaultResolver,
    source: Rc<RefCell<LayerBytes>>,
    revisions: LayerBytes,
    sealed: bool,
}

impl Resolver for SnapshotResolver {
    fn create_identifier(&self, asset_path: &str, anchor: Option<&ResolvedPath>) -> String {
        if self.sealed {
            let name = PathBuf::from(asset_path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if name.is_absolute() {
                return name.to_string_lossy().into_owned();
            }
            if let Some(directory) = anchor.and_then(|path| path.as_ref().parent()) {
                let anchored = directory.join(&name);
                if let Some(path) = self.resolve(&anchored.to_string_lossy()) {
                    return path.to_string_lossy().into_owned();
                }
                // Explicit relative paths must remain relative to their layer.
                if asset_path.starts_with("./")
                    || asset_path.starts_with("../")
                    || asset_path.starts_with(".\\")
                    || asset_path.starts_with("..\\")
                {
                    return anchored.to_string_lossy().into_owned();
                }
            }
            return name.to_string_lossy().into_owned();
        }
        self.filesystem.create_identifier(asset_path, anchor)
    }

    fn resolve(&self, asset_path: &str) -> Option<ResolvedPath> {
        // Retained names are authoritative even if their original files are
        // missing (Git materialization/relocation) or have since been replaced.
        let key = Path::new(asset_path);
        if self.revisions.contains_key(key) || self.source.borrow().contains_key(key) {
            return Some(ResolvedPath::new(key.to_owned()));
        }
        if let Some((outer, inner)) = ar::split_package_relative_path_outer(asset_path) {
            // Resolve the outer package from retained bytes, not from the live
            // filesystem; package_entry will then read its immutable buffer.
            let outer = self.resolve(&outer)?;
            let outer = outer.to_string_lossy();
            openusd::usdz::package_entry(self, &outer, &inner).ok()?;
            return Some(ResolvedPath::new(ar::join_package_relative_path(
                &outer, &inner,
            )));
        }
        if self.sealed {
            let absolute = self.filesystem.resolve_for_new_asset(asset_path)?;
            let normalized = lexical_path(absolute.as_ref());
            if let Some((path, _)) = self.revisions.get_key_value(&normalized) {
                return Some(ResolvedPath::new(path.clone()));
            }
            let source = self.source.borrow();
            if let Some((path, _)) = source.get_key_value(&normalized) {
                return Some(ResolvedPath::new(path.clone()));
            }
            #[cfg(windows)]
            {
                let spelling = normalized.to_str()?;
                if let Some(plain) = spelling.strip_prefix(r"\\?\") {
                    return source
                        .get_key_value(Path::new(plain))
                        .map(|(path, _)| ResolvedPath::new(path.clone()));
                }
                let extended = format!(r"\\?\{spelling}");
                return source
                    .get_key_value(Path::new(&extended))
                    .map(|(path, _)| ResolvedPath::new(path.clone()));
            }
            #[cfg(not(windows))]
            return None;
        }
        self.filesystem.resolve(asset_path)
    }

    fn resolve_for_new_asset(&self, asset_path: &str) -> Option<ResolvedPath> {
        self.filesystem.resolve_for_new_asset(asset_path)
    }

    fn is_context_dependent_path(&self, path: &str) -> bool {
        self.filesystem.is_context_dependent_path(path)
    }

    fn in_memory_identifier<'a>(&self, path: &'a str) -> std::borrow::Cow<'a, str> {
        self.filesystem.in_memory_identifier(path)
    }

    fn cache_id(&self) -> Option<u64> {
        self.filesystem.cache_id()
    }

    fn open_asset(&self, path: &ResolvedPath) -> io::Result<Box<dyn Asset>> {
        let key: &Path = path.as_ref();
        if let Some(bytes) = self.revisions.get(key).cloned() {
            return Ok(Box::new(Cursor::new(AssetBuffer::Shared(bytes))));
        }
        if let Some(bytes) = self.source.borrow().get(key).cloned() {
            return Ok(Box::new(Cursor::new(AssetBuffer::Shared(bytes))));
        }
        let bytes = if let Some((outer, inner)) =
            ar::split_package_relative_path_outer(&path.to_string_lossy())
        {
            openusd::usdz::package_entry(self, &outer, &inner).map_err(io::Error::other)?
        } else {
            if self.sealed {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("USD asset {path} is outside the retained source closure"),
                ));
            }
            self.filesystem.open_asset(path)?.into_buffer()?
        }
        .into_shared();
        self.source
            .borrow_mut()
            .insert(key.to_owned(), bytes.clone());
        Ok(Box::new(Cursor::new(AssetBuffer::Shared(bytes))))
    }
}

fn lexical_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            item => result.push(item.as_os_str()),
        }
    }
    result
}

impl VisualStage {
    pub(super) fn texture_source(&self, stage_identity: (u64, u64)) -> SnapshotTextures {
        SnapshotTextures {
            stage_identity,
            source: self.source.clone(),
        }
    }

    pub(super) fn open_pair(path: &Path) -> Result<(Stage, Self)> {
        let path = path.canonicalize().context("resolve USD document entry")?;
        // Discovery is staged and never installed. The two published stages
        // use independent registries over exactly the same sealed source bytes.
        let source = capture_source_snapshot(&path)?;
        let stage = open(&path, &source, HashMap::new(), true)?;
        validate_composition(&stage)?;
        let visual = open(&path, &source, HashMap::new(), true)?;
        validate_composition(&visual)?;
        Ok((
            stage,
            Self {
                path,
                source,
                revisions: HashMap::new(),
                stage: visual,
                authoring_revision: LiveRevision::default(),
            },
        ))
    }

    /// Publish a new visual stage only after its full visible composition
    /// succeeds. Keep the existing visual payload policy across authoring.
    pub(super) fn synchronize(&mut self, canonical: &Stage, revision: LiveRevision) -> Result<()> {
        if self.authoring_revision == revision {
            return Ok(());
        }
        validate_composition(canonical)?;
        let mut revisions = self.revisions.clone();
        for identifier in canonical.layer_identifiers() {
            let Some(layer) = canonical.layer(&identifier)? else {
                continue;
            };
            if layer.try_borrow()?.is_dirty() {
                let path = layer.resolved_path().with_context(|| {
                    format!("cannot capture unsaved opinions in memory-only layer {identifier}")
                })?;
                let bytes = SharedBuffer::new(layer.export_to_string()?.into_bytes());
                revisions.insert(PathBuf::from(path), bytes);
            }
        }
        let stage = open(&self.path, &self.source, revisions.clone(), true)?;
        stage.set_load_rules(self.stage.load_rules());
        validate_composition(&stage)?;
        self.stage = stage;
        self.revisions = revisions;
        self.authoring_revision = revision;
        Ok(())
    }
}

pub(super) fn open_read_only(path: &Path) -> Result<Stage> {
    let path = path
        .canonicalize()
        .context("resolve authoritative USD entry")?;
    let snapshot = capture_source_snapshot(&path)?;
    let stage = open(&path, &snapshot, HashMap::new(), true)?;
    validate_composition(&stage)?;
    Ok(stage)
}

pub(super) fn open_retained(
    path: &Path,
    source: impl IntoIterator<Item = (PathBuf, Vec<u8>)>,
) -> Result<Stage> {
    let source = source
        .into_iter()
        .map(|(path, bytes)| (path, SharedBuffer::new(bytes)))
        .collect();
    let stage = open_with_resolver(
        path,
        SnapshotResolver {
            filesystem: DefaultResolver::with_search_paths(path.parent()),
            source: Rc::new(RefCell::new(source)),
            revisions: HashMap::new(),
            sealed: true,
        },
    )?;
    validate_composition(&stage)?;
    Ok(stage)
}

/// Capture the exact native dependency graph *before* either published stage
/// exists. Do not treat a Stage::open (which can lazily read assets) as a
/// complete snapshot. All late reads of uncaptured files must fail closed.
/// Unsupported native dependencies (e.g. an unexpanded clip template) abort
/// the candidate open rather than silently mixing filesystem revisions.
fn capture_source_snapshot(path: &Path) -> Result<Rc<RefCell<LayerBytes>>> {
    let source = Rc::new(RefCell::new(HashMap::new()));
    let inspection = open(path, &source, HashMap::new(), false)?;
    validate_composition(&inspection)?;
    let identifier = path
        .to_str()
        .context("USD snapshot path must be valid UTF-8")?;
    let dependencies = all_dependencies(&inspection, identifier)
        .context("discover complete immutable USD source dependencies")?;

    // Use this same resolver for every native path, so package/anchor rules
    // agree with the staged inspection. The staged Stage is discarded.
    let resolver = SnapshotResolver {
        filesystem: DefaultResolver::new(),
        source: source.clone(),
        revisions: HashMap::new(),
        sealed: false,
    };
    let _scope = ar::CacheScope::begin();
    let mut required = BTreeSet::new();
    required.insert(identifier.to_owned());
    required.extend(dependencies.layers);
    required.extend(dependencies.assets);
    for asset in required {
        let resolved = resolver
            .resolve(&asset)
            .with_context(|| format!("USD source dependency no longer resolves: {asset}"))?;
        resolver
            .open_asset(&resolved)
            .with_context(|| format!("capture USD source dependency: {asset}"))?;
    }
    drop(inspection);

    // Detect a file being replaced during capture, rather than publishing an
    // internally inconsistent mix. Package-entry keys are virtual: verifying
    // their captured outer archive already covers their encoded source.
    for (name, expected) in source.borrow().iter() {
        if ar::split_package_relative_path_outer(&name.to_string_lossy()).is_some() {
            continue;
        }
        let mut current = std::fs::File::open(name)
            .with_context(|| format!("revalidate USD source snapshot {}", name.display()))?;
        let mut buffer = [0; 64 * 1024];
        for chunk in expected.as_ref().chunks(buffer.len()) {
            ensure!(
                current.read_exact(&mut buffer[..chunk.len()]).is_ok()
                    && buffer[..chunk.len()] == *chunk,
                "USD source changed during immutable snapshot capture: {}",
                name.display()
            );
        }
        ensure!(
            current.read(&mut buffer[..1])? == 0,
            "USD source grew during immutable snapshot capture: {}",
            name.display()
        );
    }
    Ok(source)
}

fn open(
    path: &Path,
    source: &Rc<RefCell<LayerBytes>>,
    revisions: LayerBytes,
    sealed: bool,
) -> Result<Stage> {
    open_with_resolver(
        path,
        SnapshotResolver {
            filesystem: DefaultResolver::new(),
            source: source.clone(),
            revisions,
            sealed,
        },
    )
}

fn open_with_resolver(path: &Path, resolver: SnapshotResolver) -> Result<Stage> {
    let identifier = path
        .to_str()
        .context("USD stage path must be valid UTF-8")?;
    Ok(Stage::builder()
        .resolver(resolver)
        .schema_registry(openusd_schemas::schema_registry())
        .load(InitialLoadSet::LoadAll)
        .open(identifier)?)
}

fn validate_composition(stage: &Stage) -> Result<()> {
    stage.traverse(PrimPredicate::ALL, |path| {
        if let Ok(prim) = stage.prim(path) {
            let _ = prim.prim_index().graph();
        }
    })?;
    let errors = stage.composition_errors();
    ensure!(errors.is_empty(), "USD composition errors: {errors:?}");
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/live/visual_stage_snapshot.rs"]
mod tests;
