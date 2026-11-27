use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
type IdentifierKey = (String, Option<PathBuf>);

/// Resolver observations are recorded only during discovery. Once the source
/// is sealed, all resolver instances share these same immutable associations.
#[derive(Default)]
struct SourceBindings {
    files: BTreeMap<PathBuf, PathBuf>,
    observed: BTreeMap<String, PathBuf>,
    identifiers: BTreeMap<IdentifierKey, String>,
    short_prefixes: BTreeMap<PathBuf, PathBuf>,
    directories: BTreeSet<PathBuf>,
    #[cfg(windows)]
    inspected_short_prefixes: BTreeSet<PathBuf>,
    conflict: bool,
    incomplete: bool,
}

impl SourceBindings {
    fn insert_file(&mut self, spelling: &Path, resolved: &Path) -> bool {
        let key = binding_key(spelling);
        if self.files.get(&key).is_some_and(|existing| existing != resolved) {
            self.conflict = true;
            return false;
        }
        self.files.insert(key, resolved.to_owned());
        self.directories.extend(resolved.ancestors().skip(1).map(binding_key));
        true
    }

    fn observe(&mut self, spelling: &str, resolved: &Path) -> bool {
        if self.observed.get(spelling).is_some_and(|existing| existing != resolved) {
            self.conflict = true;
            return false;
        }
        self.observed.insert(spelling.to_owned(), resolved.to_owned());
        self.insert_file(Path::new(spelling), resolved) && self.insert_file(resolved, resolved)
    }

    fn record_identifier(&mut self, key: IdentifierKey, identifier: String) {
        if self.identifiers.get(&key).is_some_and(|old| old != &identifier) {
            self.conflict = true;
        } else {
            self.identifiers.insert(key, identifier);
        }
    }

    fn resolve(&self, spelling: &Path) -> Option<&PathBuf> {
        let key = binding_key(spelling);
        if let Some(path) = self.files.get(&key) {
            return Some(path);
        }
        // Translate only observed short components, not live OS aliases. Walk
        // from the root so mixed short/long spellings normalize consistently.
        let mut path = PathBuf::new();
        for component in key.components() {
            match component {
                std::path::Component::ParentDir => {
                    // A parent step is a lexical equivalent only after a
                    // captured physical directory, never after an unknown alias.
                    if !self.directories.contains(&path) || !path.pop() {
                        return None;
                    }
                }
                std::path::Component::CurDir => {}
                item => path.push(item.as_os_str()),
            }
            if let Some(prefix) = self.short_prefixes.get(&path) {
                path = prefix.clone();
            }
        }
        self.files.get(&path)
    }

    fn identity_bytes(&self) -> Result<Vec<u8>> {
        ensure!(!self.conflict && !self.incomplete, "incomplete or conflicting sealed USD resolver bindings");
        fn append(bytes: &mut Vec<u8>, value: &str) {
            bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        fn append_path(bytes: &mut Vec<u8>, path: &Path) -> Result<()> {
            append(bytes, path.to_str().context("non-UTF8 sealed resolver binding")?);
            Ok(())
        }
        let mut bytes = b"nemeton.captured-resolver-bindings.v1\0".to_vec();
        for map in [&self.files, &self.short_prefixes] {
            bytes.extend_from_slice(&(map.len() as u64).to_le_bytes());
            for (key, value) in map {
                append_path(&mut bytes, key)?;
                append_path(&mut bytes, value)?;
            }
        }
        bytes.extend_from_slice(&(self.identifiers.len() as u64).to_le_bytes());
        for ((asset, anchor), identifier) in &self.identifiers {
            append(&mut bytes, asset);
            bytes.push(u8::from(anchor.is_some()));
            if let Some(anchor) = anchor {
                append_path(&mut bytes, anchor)?;
            }
            append(&mut bytes, identifier);
        }
        Ok(bytes)
    }
}

/// Normalize equivalent lexical spellings without consulting the filesystem.
fn binding_key(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(path) = path.to_str() {
        if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            return lexical_path(Path::new(&format!(r"\\{unc}")));
        }
        if let Some(plain) = path.strip_prefix(r"\\?\") {
            // Only DOS drive paths have a corresponding ordinary spelling.
            if plain.as_bytes().get(1) == Some(&b':') {
                return lexical_path(Path::new(plain));
            }
        }
    }
    #[cfg(not(windows))]
    if !path.is_absolute() && let Some(path) = path.to_str() {
        return lexical_path(Path::new(&path.replace('\\', "/")));
    }
    lexical_path(path)
}

/// Capture actual Windows 8.3 component spellings while the source is still
/// being qualified. No short-name or other OS lookup is performed after seal.
#[cfg(windows)]
fn capture_short_prefixes(bindings: &mut SourceBindings, path: &Path) -> Result<()> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetShortPathNameW(long_path: *const u16, short_path: *mut u16, capacity: u32) -> u32;
    }

    for prefix in path.ancestors().filter(|prefix| prefix.file_name().is_some()) {
        if !bindings.inspected_short_prefixes.insert(prefix.to_owned()) {
            continue;
        }
        let mut wide: Vec<u16> = prefix.as_os_str().encode_wide().collect();
        ensure!(!wide.contains(&0), "invalid NUL in USD source path");
        wide.push(0);
        // SAFETY: wide is NUL-terminated; a null output and zero capacity query
        // the size without writing through that pointer.
        let capacity = unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) };
        if capacity == 0 {
            // Short names are optional on Windows volumes. Observed aliases
            // remain captured; an unobserved unsupported spelling is a miss.
            continue;
        }
        let mut output = vec![0u16; capacity as usize];
        // SAFETY: output has capacity writable UTF-16 elements, wide remains
        // NUL-terminated and both buffers live through the synchronous call.
        let written = unsafe { GetShortPathNameW(wide.as_ptr(), output.as_mut_ptr(), capacity) };
        ensure!(written > 0 && written < capacity, "Windows short-name mapping changed during capture");
        let short = PathBuf::from(std::ffi::OsString::from_wide(&output[..written as usize]));
        let Some(short_name) = short.file_name() else { continue; };
        let Some(parent) = prefix.parent() else { continue; };
        let canonical = binding_key(prefix);
        let alias_path = parent.join(short_name);
        let alias = binding_key(&alias_path);
        if alias == canonical { continue; }
        ensure!(alias_path.canonicalize()?.as_path() == prefix, "Windows short-name mapping changed during USD capture");
        if let Some(existing) = bindings.short_prefixes.get(&alias) {
            ensure!(existing == &canonical, "conflicting captured Windows short-name mapping");
        } else {
            bindings.short_prefixes.insert(alias, canonical);
        }
    }
    Ok(())
}


pub(crate) struct SnapshotTextures {
    pub(super) stage_identity: (u64, u64),
    missing_required_texture: Rc<Cell<bool>>,
    source: Rc<RefCell<LayerBytes>>,
    bindings: Rc<RefCell<SourceBindings>>,
}

impl SnapshotTextures {
    pub(crate) fn read(&self, path: &str) -> Option<Vec<u8>> {
        let resolver = SnapshotResolver {
            filesystem: DefaultResolver::new(),
            source: self.source.clone(),
            bindings: self.bindings.clone(),
            revisions: HashMap::new(),
            sealed: true,
        };
        let result = resolver.resolve(path)
            .and_then(|resolved| resolver.open_asset(&resolved).ok())
            .and_then(|asset| asset.into_buffer().ok())
            .map(|bytes| bytes.as_ref().to_vec());
        if result.is_none() {
            self.missing_required_texture.set(true);
        }
        result
    }
}

/// A file-backed document's projection context. OpenUSD owns composition and
/// payload rules in each stage. Only immutable encoded inputs cross registries.
pub(super) struct VisualStage {
    path: PathBuf,
    missing_required_texture: Rc<Cell<bool>>,
    source: Rc<RefCell<LayerBytes>>,
    bindings: Rc<RefCell<SourceBindings>>,
    revisions: LayerBytes,
    session_path: PathBuf,
    pub(super) stage: Stage,
    pub(super) authoring_revision: LiveRevision,
}

struct SnapshotResolver {
    filesystem: DefaultResolver,
    source: Rc<RefCell<LayerBytes>>,
    bindings: Rc<RefCell<SourceBindings>>,
    revisions: LayerBytes,
    sealed: bool,
}

impl SnapshotResolver {
    /// DefaultResolver can canonicalize an authored path in create_identifier
    /// before resolve sees it. Record the original absolute/anchored spelling
    /// at this same unsealed boundary, not just the returned canonical name.
    fn capture_identifier_alias(&self, asset_path: &str, anchor: Option<&ResolvedPath>, identifier: &str) {
        if asset_path.is_empty() || ar::split_package_relative_path_outer(asset_path).is_some()
            || !Path::new(identifier).is_absolute()
        {
            return;
        }
        let Some(resolved) = self.filesystem.resolve(identifier) else { return; };
        let raw = Path::new(asset_path);
        let candidate = if raw.is_absolute() {
            raw.to_owned()
        } else {
            let package = anchor.and_then(|path| {
                ar::split_package_relative_path_outer(&path.to_string_lossy())
                    .map(|(outer, _)| PathBuf::from(outer))
            });
            let directory = package.as_deref().or(anchor.map(|path| path.as_ref()))
                .and_then(Path::parent).map(Path::to_path_buf)
                .or_else(|| std::env::current_dir().ok());
            let Some(directory) = directory else { return; };
            directory.join(binding_key(raw))
        };
        let Some(spelling) = candidate.to_str() else {
            self.bindings.borrow_mut().conflict = true;
            return;
        };
        let mut bindings = self.bindings.borrow_mut();
        if self.filesystem.resolve(spelling).as_ref() != Some(&resolved) {
            bindings.conflict = true;
            return;
        }
        bindings.observe(spelling, resolved.as_ref());
    }
}

impl Resolver for SnapshotResolver {
    fn create_identifier(&self, asset_path: &str, anchor: Option<&ResolvedPath>) -> String {
        let key = (asset_path.to_owned(), anchor.map(|path| path.as_ref().to_path_buf()));
        if self.sealed {
            if let Some(identifier) = self.bindings.borrow().identifiers.get(&key) {
                return identifier.clone();
            }
            if let Some((outer, inner)) = ar::split_package_relative_path_outer(asset_path) {
                let outer = self.create_identifier(&outer, anchor);
                return ar::join_package_relative_path(&outer, &inner);
            }
            let raw = Path::new(asset_path);
            if raw.is_absolute() {
                return asset_path.to_owned();
            }
            let name = binding_key(raw);
            let package = anchor.and_then(|path| {
                ar::split_package_relative_path_outer(&path.to_string_lossy())
                    .map(|(outer, _)| PathBuf::from(outer))
            });
            if let Some(directory) = package.as_deref().or(anchor.map(|path| path.as_ref())).and_then(Path::parent) {
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
        let identifier = self.filesystem.create_identifier(asset_path, anchor);
        self.capture_identifier_alias(asset_path, anchor, &identifier);
        self.bindings.borrow_mut().record_identifier(key, identifier.clone());
        identifier
    }

    fn resolve(&self, asset_path: &str) -> Option<ResolvedPath> {
        // Retained names are authoritative even if their original files are
        // missing (Git materialization/relocation) or have since been replaced.
        let key = Path::new(asset_path);
        if self.revisions.contains_key(key) || self.source.borrow().contains_key(key) {
            if !self.sealed && ar::split_package_relative_path_outer(asset_path).is_none()
                && !self.bindings.borrow_mut().observe(asset_path, key)
            {
                return None;
            }
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
            let normalized = binding_key(key);
            if let Some((path, _)) = self.revisions.iter().find(|(path, _)| binding_key(path) == normalized) {
                return Some(ResolvedPath::new(path.clone()));
            }
            let bindings = self.bindings.borrow();
            let resolved = bindings.resolve(key)?;
            return (self.source.borrow().contains_key(resolved) || self.revisions.contains_key(resolved))
                .then(|| ResolvedPath::new(resolved.clone()));
        }
        let resolved = self.filesystem.resolve(asset_path)?;
        self.bindings.borrow_mut().observe(asset_path, resolved.as_ref()).then_some(resolved)
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
            // Preserve parent steps until lookup can prove their prefixes
            // refer to captured physical directories.
            item => result.push(item.as_os_str()),
        }
    }
    result
}

impl VisualStage {
    pub(super) fn opened_root_bytes(&self) -> Option<SharedBuffer> {
        self.source.borrow().get(&self.path).cloned()
    }

    pub(super) fn opened_source_path(&self) -> &Path {
        &self.path
    }

    pub(super) fn opened_source_files(&self) -> std::collections::BTreeMap<PathBuf, SharedBuffer> {
        self.source.borrow().iter()
            .filter(|(path, _)| ar::split_package_relative_path_outer(&path.to_string_lossy()).is_none())
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .collect()
    }
    pub(super) fn opened_resolver_bindings(&self) -> Result<Vec<u8>> {
        ensure!(!self.missing_required_texture.get(), "sealed texture input unavailable");
        self.bindings.borrow().identity_bytes()
    }

    pub(super) fn texture_source(&self, stage_identity: (u64, u64)) -> SnapshotTextures {
        SnapshotTextures {
            stage_identity,
            missing_required_texture: self.missing_required_texture.clone(),
            source: self.source.clone(),
            bindings: self.bindings.clone(),
        }
    }

    pub(super) fn open_pair(path: &Path) -> Result<(Stage, Self)> {
        let path = path.canonicalize().context("resolve USD document entry")?;
        // Discovery is staged and never installed. The two published stages
        // use independent registries over exactly the same sealed source bytes.
        let CapturedSource { source, bindings } = capture_source_snapshot(&path)?;
        let stage = open(&path, &source, &bindings, HashMap::new(), true)?;
        validate_composition(&stage)?;
        let session_path = path.with_extension("nemeton_visual_session.usda");
        ensure!(
            !source.borrow().contains_key(&session_path),
            "visual session name collides with a source layer"
        );
        let visual = open_visual(&path, &source, &bindings, HashMap::new(), &session_path)?;
        validate_composition(&visual)?;
        Ok((
            stage,
            Self {
                path,
                missing_required_texture: Rc::new(Cell::new(false)),
                source,
                bindings,
                revisions: HashMap::new(),
                session_path,
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
        // Re-evaluate camera overrides against the newly authored variant
        // topology. Replaying old opinions could conceal an author's edit.
        let stage = open_visual(
            &self.path,
            &self.source,
            &self.bindings,
            revisions.clone(),
            &self.session_path,
        )?;
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
    let CapturedSource { source, bindings } = capture_source_snapshot(&path)?;
    let stage = open(&path, &source, &bindings, HashMap::new(), true)?;
    validate_composition(&stage)?;
    Ok(stage)
}

pub(super) fn open_retained(
    path: &Path,
    source: impl IntoIterator<Item = (PathBuf, Vec<u8>)>,
) -> Result<Stage> {
    let source: LayerBytes = source
        .into_iter()
        .map(|(path, bytes)| (path, SharedBuffer::new(bytes)))
        .collect();
    let mut bindings = SourceBindings::default();
    for path in source.keys() {
        if ar::split_package_relative_path_outer(&path.to_string_lossy()).is_none() {
            ensure!(bindings.insert_file(path, path), "conflicting retained source paths");
        }
    }
    let stage = open_with_resolver(
        path,
        SnapshotResolver {
            filesystem: DefaultResolver::new(),
            source: Rc::new(RefCell::new(source)),
            bindings: Rc::new(RefCell::new(bindings)),
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
struct CapturedSource {
    source: Rc<RefCell<LayerBytes>>,
    bindings: Rc<RefCell<SourceBindings>>,
}

fn capture_source_snapshot(path: &Path) -> Result<CapturedSource> {
    let source = Rc::new(RefCell::new(HashMap::new()));
    let bindings = Rc::new(RefCell::new(SourceBindings::default()));
    let inspection = open(path, &source, &bindings, HashMap::new(), false)?;
    validate_composition(&inspection)?;
    let identifier = path
        .to_str()
        .context("USD snapshot path must be valid UTF-8")?;
    let dependencies = all_dependencies(&inspection, identifier)
        .context("discover complete immutable USD source dependencies")?;
    // Native discovery reports unresolved assets separately. Ordinary source
    // opening can still display fallback materials; persistent Ready cannot
    // claim a complete resolver closure for an unresolved required input.
    bindings.borrow_mut().incomplete = !dependencies.unresolved.is_empty();

    // Use this same resolver for every native path, so package/anchor rules
    // agree with the staged inspection. The staged Stage is discarded.
    let resolver = SnapshotResolver {
        filesystem: DefaultResolver::new(),
        source: source.clone(),
        bindings: bindings.clone(),
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
    // Native discovery may resolve an identifier while probing a dependency
    // without opening the resulting file. Every accepted binding must have
    // a retained destination, including such successful probes.
    let observed: BTreeSet<_> = bindings.borrow().observed.values().cloned().collect();
    for path in observed {
        if !source.borrow().contains_key(&path) {
            resolver.open_asset(&ResolvedPath::new(path))
                .context("capture observed USD resolver destination")?;
        }
    }

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
    {
        let mut captured = bindings.borrow_mut();
        ensure!(!captured.conflict, "conflicting USD source bindings during capture");
        // Check the actual unsealed resolver, not the retained-map fast path.
        // This verifies the alias association as well as the bytes above.
        for (spelling, resolved) in &captured.observed {
            let current = resolver.filesystem.resolve(spelling)
                .with_context(|| format!("USD alias no longer resolves: {spelling}"))?;
            ensure!(current.as_ref() == resolved, "USD alias changed during capture: {spelling}");
        }
        for ((asset, anchor), identifier) in &captured.identifiers {
            let anchor = anchor.as_ref().map(|path| ResolvedPath::new(path.clone()));
            ensure!(resolver.filesystem.create_identifier(asset, anchor.as_ref()) == *identifier,
                "USD identifier changed during capture: {asset}");
        }
        for path in source.borrow().keys() {
            if ar::split_package_relative_path_outer(&path.to_string_lossy()).is_some() {
                continue;
            }
            ensure!(captured.insert_file(path, path), "conflicting captured USD source path");
            #[cfg(windows)]
            capture_short_prefixes(&mut captured, path)?;
        }
        for resolved in captured.files.values() {
            ensure!(source.borrow().contains_key(resolved), "resolved USD asset not retained: {}", resolved.display());
        }
    }
    Ok(CapturedSource { source, bindings })
}

fn open(
    path: &Path,
    source: &Rc<RefCell<LayerBytes>>,
    bindings: &Rc<RefCell<SourceBindings>>,
    revisions: LayerBytes,
    sealed: bool,
) -> Result<Stage> {
    open_with_resolver(
        path,
        SnapshotResolver {
            filesystem: DefaultResolver::new(),
            source: source.clone(),
            bindings: bindings.clone(),
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

fn open_visual(
    path: &Path,
    source: &Rc<RefCell<LayerBytes>>,
    bindings: &Rc<RefCell<SourceBindings>>,
    mut revisions: LayerBytes,
    session_path: &Path,
) -> Result<Stage> {
    let identifier = path
        .to_str()
        .context("USD stage path must be valid UTF-8")?;
    let session_name = session_path
        .to_str()
        .context("visual session path must be UTF-8")?;
    revisions.insert(
        session_path.to_owned(),
        SharedBuffer::new(b"#usda 1.0\n".to_vec()),
    );
    Ok(Stage::builder()
        .resolver(SnapshotResolver {
            filesystem: DefaultResolver::new(),
            source: source.clone(),
            bindings: bindings.clone(),
            revisions,
            sealed: true,
        })
        .schema_registry(openusd_schemas::schema_registry())
        .session_layer(session_name)
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
