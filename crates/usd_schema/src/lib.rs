//! Schema-authoring helpers layered on top of `openusd::sdf::Data`.
//!
//! `openusd` is schema-agnostic — it exposes the Sdf (spec) layer only,
//! with no typed `UsdGeom::Mesh`, `UsdPhysics::RigidBodyAPI`, or
//! `UsdShade::Material` builders. This crate is the thin convenience
//! layer you would otherwise write by hand for every tool that needs
//! to author composed USD from Rust: it stamps the right `TypeName`
//! tokens, `apiSchemas` list ops, relationships, and attribute
//! defaults that Pixar's C++ schema classes would emit.
//!
//! Modules:
//! - [`xform`] — `Xformable` transform ops (translate / orient / scale).
//! - [`geom`] — `UsdGeom.{Cube, Sphere, Cylinder, Capsule, Mesh,
//!   GeomSubset}` primitives.
//! - [`physics`] — `UsdPhysics.{Scene, RigidBodyAPI, MassAPI,
//!   CollisionAPI, MeshCollisionAPI, *Joint, LimitAPI,
//!   ArticulationRootAPI}` authoring plus `NewtonMimicAPI` for the
//!   Newton ecosystem.
//! - [`shade`] — `UsdShade.{Material, Shader}` with a full
//!   `UsdPreviewSurface` + per-channel `UsdUVTexture` graph and the
//!   PreviewMaterial input-interface promotion pattern.
//! - [`tokens`] — string constants for every schema name we author.
//! - [`math`] — tiny math helpers used by `xform` (RPY → quaternion).
//!
//! Errors surface through `anyhow::Error` so callers can adapt freely.

pub mod anim;
pub mod camera;
pub mod clips;
pub mod geom;
pub mod lux;
pub mod math;
pub mod media;
pub mod physics;
pub mod proc;
pub mod render;
pub mod shade;
pub mod skel;
/// Sidecar text-mode parser for `UsdSkelAnimation` prims authored in
/// USDA. Pixar's `HumanFemale.walk.usd` (and most production walk
/// cycles) author tuple-valued time samples for `quatf[] rotations`,
/// `float3[] translations`, and `half3[] scales` that openusd-rs's
/// USDA parser currently rejects (`Unsupported property metadata
/// value token: Punctuation('(')`). This module reads the raw .usda
/// text and extracts the SkelAnimation samples directly so we can
/// drive joint transforms while the upstream parser is fixed.
pub mod skel_anim_text;
pub mod tokens;
pub mod ui;
pub mod xform;

/// Glue + workarounds for the third-party `openusd-rs` crate that
/// don't fit the schema-authoring story the rest of this crate is
/// about — strip-metadata preprocess, a strip-aware `Resolver`, and
/// MDL → UsdPreviewSurface conversion. Lives under the `3rd_party/`
/// folder on disk; `#[path]` lets Rust accept the digit-leading
/// folder name while exposing it as the valid identifier
/// `third_party`.
#[path = "3rd_party/mod.rs"]
pub mod third_party;

use std::collections::HashMap;
use std::path::Path as StdPath;

use anyhow::{Result, anyhow};
use openusd::sdf::{
    self, AbstractData, ChildrenKey, FieldKey, ListOp, Path, SpecType, Specifier, Value,
};

/// Compatibility shim for schema readers while using OpenUSD 0.5's public,
/// typed composed-scene handles. This keeps all legacy field routing in one
/// place instead of reaching into the now-private `usd::Stage::field` API.
pub trait StageReadExt {
    fn composed_field<T: FromComposedValue>(
        &self,
        path: impl Into<Path>,
        field: impl AsRef<str>,
    ) -> Result<Option<T>>;

    /// Composed child prim names for `path`.
    fn prim_children(&self, path: impl Into<Path>) -> Result<Vec<String>>;

    /// Composed property names for `path`.
    fn prim_properties(&self, path: impl Into<Path>) -> Result<Vec<String>>;

    /// Composed applied API schema names for `path`.
    fn api_schemas(&self, path: impl Into<Path>) -> Result<Vec<String>>;

    /// Whether a composed prim exists at `path`.
    fn prim_exists(&self, path: impl Into<Path>) -> Result<bool>;
}

pub trait FromComposedValue: Sized {
    fn from_composed_value(value: Value) -> Result<Self>;
}

impl FromComposedValue for Value {
    fn from_composed_value(value: Value) -> Result<Self> {
        Ok(value)
    }
}

impl FromComposedValue for String {
    fn from_composed_value(value: Value) -> Result<Self> {
        match value {
            Value::String(value) => Ok(value),
            Value::Token(value) => Ok(value.to_string()),
            Value::AssetPath(value) => Ok(value.into_string()),
            other => Err(anyhow!("expected string-like USD value, got {other:?}")),
        }
    }
}

impl FromComposedValue for bool {
    fn from_composed_value(value: Value) -> Result<Self> {
        match value {
            Value::Bool(value) => Ok(value),
            other => Err(anyhow!("expected bool USD value, got {other:?}")),
        }
    }
}

pub(crate) fn value_into_string(value: Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value),
        Value::Token(value) => Some(value.to_string()),
        Value::AssetPath(value) => Some(value.into_string()),
        _ => None,
    }
}

pub(crate) fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Token(value) => Some(value.to_string()),
        Value::AssetPath(value) => Some(value.to_string()),
        _ => None,
    }
}

pub(crate) fn value_into_string_vec(value: Value) -> Option<Vec<String>> {
    match value {
        Value::StringVec(value) => Some(value),
        Value::TokenVec(value) => Some(value.into_iter().map(|token| token.to_string()).collect()),
        _ => None,
    }
}

pub(crate) fn value_to_string_vec(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::StringVec(value) => Some(value.clone()),
        Value::TokenVec(value) => Some(value.iter().map(|token| token.to_string()).collect()),
        _ => None,
    }
}

impl StageReadExt for openusd::usd::Stage {
    fn composed_field<T: FromComposedValue>(
        &self,
        path: impl Into<Path>,
        field: impl AsRef<str>,
    ) -> Result<Option<T>> {
        let path = path.into();
        let field = field.as_ref();
        let value = if path.is_property_path() {
            let attribute = self.attribute(path.clone());
            match field {
                "default" => attribute.get::<Value>()?,
                "timeSamples" => attribute.time_samples()?.map(Value::TimeSamples),
                "connectionPaths" => Some(Value::PathListOp(sdf::PathListOp::explicit(
                    attribute.connections()?,
                ))),
                "targetPaths" => Some(Value::PathListOp(sdf::PathListOp::explicit(
                    self.relationship(path.clone()).targets()?,
                ))),
                "custom" => Some(Value::Bool(attribute.is_custom()?)),
                "typeName" => attribute.type_name()?.map(Value::Token),
                _ => attribute.get_metadata::<Value>(field)?,
            }
        } else {
            let prim = self.prim(path.clone());
            match field {
                "typeName" => prim.type_name()?.map(Value::Token),
                "kind" => prim.kind()?.map(Value::Token),
                "customData" => prim.custom_data()?,
                _ => {
                    let mut found = None;
                    if path == Path::abs_root() {
                        found = self
                            .root_layer()
                            .data()
                            .try_field(&path, field)?
                            .map(|value| value.into_owned());
                    } else if path.is_prim_variant_selection_path() {
                        // A `Prim` handle normalizes to the prim path and
                        // therefore cannot expose fields authored on the
                        // variant-set container itself. Query the composed
                        // layer stack at the exact variant-selection path.
                        for layer_id in self.layer_identifiers() {
                            let Some(layer) = self.layer(&layer_id) else {
                                continue;
                            };
                            if let Some(value) = layer.data().try_field(&path, field)? {
                                found = Some(value.into_owned());
                                break;
                            }
                        }
                    } else {
                        for (layer_id, spec_path) in prim.prim_stack()? {
                            let Some(layer) = self.layer(&layer_id) else {
                                continue;
                            };
                            if let Some(value) = layer.data().try_field(&spec_path, field)? {
                                found = Some(value.into_owned());
                                break;
                            }
                        }
                    }
                    found
                }
            }
        };
        value.map(T::from_composed_value).transpose()
    }

    fn prim_children(&self, path: impl Into<Path>) -> Result<Vec<String>> {
        Ok(self
            .prim(path)
            .child_names()?
            .into_iter()
            .map(|name| name.to_string())
            .collect())
    }

    fn prim_properties(&self, path: impl Into<Path>) -> Result<Vec<String>> {
        Ok(self
            .prim(path)
            .property_names()?
            .into_iter()
            .map(|name| name.to_string())
            .collect())
    }

    fn api_schemas(&self, path: impl Into<Path>) -> Result<Vec<String>> {
        Ok(self
            .prim(path)
            .api_schemas()?
            .into_iter()
            .map(|name| name.to_string())
            .collect())
    }

    fn prim_exists(&self, path: impl Into<Path>) -> Result<bool> {
        self.prim(path).is_valid()
    }
}

/// In-memory USD stage being authored.
pub struct Stage {
    data: sdf::Data,
    /// `parent_path -> ordered list of child prim names`.
    prim_children: HashMap<Path, Vec<String>>,
    /// `prim_path -> ordered list of authored property names`.
    prop_children: HashMap<Path, Vec<String>>,
}

impl Stage {
    /// Create a new stage with a default prim named `default_prim`. The default
    /// prim is defined as a `Xform` at `/<default_prim>`, and the pseudo-root
    /// carries layer-level metadata (upAxis, metersPerUnit, etc.).
    pub fn new(default_prim: &str) -> Result<Self> {
        let mut s = Self::new_sublayer();
        let root = Path::abs_root();
        let root_spec = s.data.spec_mut(&root).expect("pseudo-root exists");
        root_spec.add(FieldKey::DefaultPrim, Value::Token(default_prim.into()));
        root_spec.add("upAxis", Value::Token("Z".into()));
        root_spec.add("metersPerUnit", Value::Double(1.0));
        root_spec.add("kilogramsPerUnit", Value::Double(1.0));

        let prim_path = child_path(&root, default_prim)?;
        s.define_prim_spec(&prim_path, tokens::T_XFORM);
        s.register_prim_child(&root, default_prim);
        Ok(s)
    }

    /// Create a blank stage suitable for use as a USD sublayer — a bare
    /// pseudo-root with no stage metadata and no default prim. Callers
    /// populate it with prim / over specs directly.
    pub fn new_sublayer() -> Self {
        let mut s = Self {
            data: sdf::Data::new(),
            prim_children: HashMap::new(),
            prop_children: HashMap::new(),
        };
        s.data.create_spec(Path::abs_root(), SpecType::PseudoRoot);
        s
    }

    /// Define a new prim at `parent/name` with the given USD type name (e.g.
    /// `"Xform"`, `"Mesh"`, `"PhysicsScene"`). Returns the prim path.
    pub fn define_prim(&mut self, parent: &Path, name: &str, type_name: &str) -> Result<Path> {
        if parent.as_str() != "/" {
            self.ensure_host_prim(parent)?;
        }
        let path = child_path(parent, name)?;
        self.define_prim_spec(&path, type_name);
        self.register_prim_child(parent, name);
        Ok(path)
    }

    fn define_prim_spec(&mut self, path: &Path, type_name: &str) {
        let spec = self.data.create_spec(path.clone(), SpecType::Prim);
        spec.add(FieldKey::Specifier, Value::Specifier(Specifier::Def));
        if !type_name.is_empty() {
            spec.add(FieldKey::TypeName, Value::Token(type_name.into()));
        }
    }

    /// Define an attribute under `prim` with the given name, USD type name
    /// (e.g. `"double3"`, `"point3f[]"`), and default value.
    pub fn define_attribute(
        &mut self,
        prim: &Path,
        name: &str,
        type_name: &str,
        default: Value,
        uniform: bool,
    ) -> Result<()> {
        self.ensure_host_prim(prim)?;
        let attr_path = prim.append_property(name).map_err(anyhow::Error::from)?;
        let spec = self.data.create_spec(attr_path, SpecType::Attribute);
        spec.add(FieldKey::TypeName, Value::Token(type_name.into()));
        spec.add(FieldKey::Default, default);
        if uniform {
            spec.add(
                FieldKey::Variability,
                Value::Variability(openusd::sdf::Variability::Uniform),
            );
        }
        self.register_prop_child(prim, name);
        Ok(())
    }

    /// Define a `custom` attribute — same as `define_attribute` but flagged
    /// with `custom = true`, which is how USD marks attributes outside any
    /// registered schema. We use this for `urdf:*` passthrough of URDF
    /// fields that UsdPhysics doesn't have a schema for.
    pub fn define_custom_attribute(
        &mut self,
        prim: &Path,
        name: &str,
        type_name: &str,
        default: Value,
    ) -> Result<()> {
        self.ensure_host_prim(prim)?;
        let attr_path = prim.append_property(name).map_err(anyhow::Error::from)?;
        let spec = self.data.create_spec(attr_path, SpecType::Attribute);
        spec.add(FieldKey::TypeName, Value::Token(type_name.into()));
        spec.add(FieldKey::Default, default);
        spec.add(FieldKey::Custom, Value::Bool(true));
        self.register_prop_child(prim, name);
        Ok(())
    }

    /// Register a sublayer reference on the pseudo-root. The resulting
    /// layer, when saved, emits `subLayers = [ @…@ ]`. Appended in
    /// order; USD treats earlier entries as *stronger*.
    pub fn add_sublayer(&mut self, asset_path: impl Into<String>) {
        use openusd::sdf::{LayerOffset, Value};
        let root = Path::abs_root();
        if !self.data.has_spec(&root) {
            self.data.create_spec(root.clone(), SpecType::PseudoRoot);
        }
        let spec = self.data.spec_mut(&root).expect("pseudo-root exists");
        let mut current: Vec<String> = match spec.get(FieldKey::SubLayers.as_str()).cloned() {
            Some(Value::StringVec(v)) => v,
            _ => Vec::new(),
        };
        let mut offsets: Vec<LayerOffset> =
            match spec.get(FieldKey::SubLayerOffsets.as_str()).cloned() {
                Some(Value::LayerOffsetVec(v)) => v,
                _ => Vec::new(),
            };
        current.push(asset_path.into());
        offsets.push(LayerOffset::default());
        spec.add(FieldKey::SubLayers, Value::StringVec(current));
        spec.add(FieldKey::SubLayerOffsets, Value::LayerOffsetVec(offsets));
    }

    /// Set layer-level metadata on the pseudo-root (e.g. `comment`,
    /// `customLayerData`, per-layer `kilogramsPerUnit`). Creates the
    /// pseudo-root spec if missing.
    pub fn set_layer_metadata(&mut self, key: &str, value: Value) {
        let root = Path::abs_root();
        if !self.data.has_spec(&root) {
            self.data.create_spec(root.clone(), SpecType::PseudoRoot);
        }
        let spec = self.data.spec_mut(&root).expect("pseudo-root exists");
        spec.add(key, value);
    }

    /// Set prim-level metadata (e.g. `instanceable`, `kind`). Creates an
    /// over-chain if the prim doesn't exist.
    pub fn set_prim_metadata(&mut self, prim: &Path, key: &str, value: Value) -> Result<()> {
        self.ensure_host_prim(prim)?;
        let spec = self.data.spec_mut(prim).expect("host prim exists");
        spec.add(key, value);
        Ok(())
    }

    /// Add an *internal* reference from `prim` to another prim path in the
    /// same layer. Used by the mesh library to let visual / collision `geom`
    /// sites share one canonical Mesh definition.
    ///
    /// USD's composition system pulls in all opinions from the target prim
    /// (including children like GeomSubsets) so callers only author the
    /// per-site deltas (e.g. `xformOp:scale`, material bindings).
    pub fn add_internal_reference(&mut self, prim: &Path, target: &Path) -> Result<()> {
        use openusd::sdf::{ListOp, Reference};
        self.ensure_host_prim(prim)?;
        let spec = self.data.spec_mut(prim).expect("host prim exists");
        let mut op: ListOp<Reference> = ListOp::default();
        op.prepended_items.push(Reference {
            asset_path: String::new(),
            prim_path: target.clone(),
            layer_offset: openusd::sdf::LayerOffset::IDENTITY,
            custom_data: Default::default(),
        });
        spec.add(FieldKey::References, Value::ReferenceListOp(op));
        Ok(())
    }

    /// Define a relationship under `prim`.
    pub fn define_relationship(
        &mut self,
        prim: &Path,
        name: &str,
        targets: Vec<Path>,
    ) -> Result<()> {
        self.ensure_host_prim(prim)?;
        let rel_path = prim.append_property(name).map_err(anyhow::Error::from)?;
        let spec = self.data.create_spec(rel_path, SpecType::Relationship);
        spec.add(
            FieldKey::TargetPaths,
            Value::PathListOp(explicit_list_op(targets)),
        );
        self.register_prop_child(prim, name);
        Ok(())
    }

    /// Define an attribute that has no default value but carries a
    /// `ConnectionPaths` field pointing at `target`. Used for UsdShade
    /// `outputs:surface.connect = </...>` wiring.
    pub fn define_connection(
        &mut self,
        prim: &Path,
        name: &str,
        type_name: &str,
        target: Path,
    ) -> Result<()> {
        self.ensure_host_prim(prim)?;
        let attr_path = prim.append_property(name).map_err(anyhow::Error::from)?;
        let spec = self.data.create_spec(attr_path, SpecType::Attribute);
        spec.add(FieldKey::TypeName, Value::Token(type_name.into()));
        spec.add(
            FieldKey::ConnectionPaths,
            Value::PathListOp(explicit_list_op(vec![target])),
        );
        self.register_prop_child(prim, name);
        Ok(())
    }

    /// Compute the path of an attribute under `prim`.
    pub fn attribute_path(&self, prim: &Path, name: &str) -> Result<Path> {
        prim.append_property(name).map_err(anyhow::Error::from)
    }

    /// Idempotently ensure that `path` and each of its ancestors exist in
    /// this stage as `over` specs (i.e. Specifier=Over, no TypeName). Used
    /// to overlay opinions onto prims that are `def`-ined in another layer.
    ///
    /// Walks up the path creating overs for missing ancestors and registers
    /// them as prim children so the pseudo-root serializes a coherent tree.
    pub fn define_over(&mut self, path: &Path) -> Result<()> {
        if self.data.has_spec(path) {
            return Ok(());
        }
        let parent = path.parent().unwrap_or_else(Path::abs_root);
        if parent.as_str() != "/" && !self.data.has_spec(&parent) {
            self.define_over(&parent)?;
        }
        if let Some(name) = path.name() {
            self.register_prim_child(&parent, name);
        }
        let spec = self.data.create_spec(path.clone(), SpecType::Prim);
        spec.add(FieldKey::Specifier, Value::Specifier(Specifier::Over));
        Ok(())
    }

    /// Apply a list of API schemas to a prim (stamps `apiSchemas` as a
    /// prepended token list op, which is how USD text form encodes
    /// `prepend apiSchemas = [...]`).
    pub fn apply_api_schemas(&mut self, prim: &Path, apis: &[&str]) -> Result<()> {
        if apis.is_empty() {
            return Ok(());
        }
        self.ensure_host_prim(prim)?;
        let Some(spec) = self.data.spec_mut(prim) else {
            return Err(anyhow!("apply_api_schemas: no spec at {}", prim.as_str()));
        };

        let existing = spec.get("apiSchemas").cloned();
        let mut list_op: ListOp<openusd::tf::Token> = match existing {
            Some(Value::TokenListOp(l)) => l,
            _ => ListOp::default(),
        };
        for api in apis {
            if !list_op.prepended_items.iter().any(|s| s.as_str() == *api) {
                list_op.prepended_items.push((*api).into());
            }
        }
        spec.add("apiSchemas", Value::TokenListOp(list_op));
        Ok(())
    }

    /// Write the stage to disk as `.usda`.
    pub fn write_usda(mut self, path: impl AsRef<StdPath>) -> Result<()> {
        self.flush_children();
        openusd::usda::TextWriter::write_to_file(&self.data, path).map_err(anyhow::Error::from)
    }

    /// Merge every spec + bookkeeping entry from `other` into `self`.
    ///
    /// If a prim spec exists in both, `self`'s spec takes precedence for its
    /// authored fields, but any *new* fields authored on `other` are merged
    /// in (so overlay opinions like apiSchemas / apply attrs land correctly).
    /// `primChildren` and `propertyChildren` bookkeeping merges additively.
    /// Used to squash the physics / materials overlay layers into the geom
    /// layer when `--no-layer-structure` is requested.
    pub fn merge_from(&mut self, other: Stage) -> Result<()> {
        let Stage {
            data: other_data,
            prim_children: other_prim_children,
            prop_children: other_prop_children,
        } = other;

        // Merge specs.
        let all_paths: Vec<Path> = <sdf::Data as sdf::AbstractData>::spec_paths(&other_data);
        for path in all_paths {
            let Some(src_spec) = other_data.spec(&path).cloned() else {
                continue;
            };
            if let Some(dst_spec) = self.data.spec_mut(&path) {
                // Existing spec — merge fields. Don't overwrite specifier /
                // typeName from the overlay (they're `over`/empty) on top of
                // the defining `def`.
                for (k, v) in src_spec.fields {
                    if (k == "specifier" || k == "typeName") && dst_spec.contains(&k) {
                        continue;
                    }
                    if (k == "primChildren" || k == "propertyChildren") && dst_spec.contains(&k) {
                        // Union merge for children lists.
                        if let (Some(Value::TokenVec(dst_list)), Value::TokenVec(src_list)) =
                            (dst_spec.get(&k).cloned(), v.clone())
                        {
                            let mut merged = dst_list;
                            for name in src_list {
                                if !merged.iter().any(|n| n == &name) {
                                    merged.push(name);
                                }
                            }
                            dst_spec.add(k.as_str(), Value::TokenVec(merged));
                            continue;
                        }
                    }
                    dst_spec.add(k.as_str(), v);
                }
            } else {
                // New spec — insert verbatim.
                let spec_ty = src_spec.ty;
                let new_spec = self.data.create_spec(path.clone(), spec_ty);
                for (k, v) in src_spec.fields {
                    new_spec.add(k.as_str(), v);
                }
            }
        }

        // Merge bookkeeping so flush_children picks up overlay-only children.
        for (parent, names) in other_prim_children {
            let entry = self.prim_children.entry(parent).or_default();
            for name in names {
                if !entry.iter().any(|s| *s == name) {
                    entry.push(name);
                }
            }
        }
        for (prim, names) in other_prop_children {
            let entry = self.prop_children.entry(prim).or_default();
            for name in names {
                if !entry.iter().any(|s| *s == name) {
                    entry.push(name);
                }
            }
        }
        Ok(())
    }

    fn register_prim_child(&mut self, parent: &Path, name: &str) {
        let entry = self.prim_children.entry(parent.clone()).or_default();
        if !entry.iter().any(|s| s == name) {
            entry.push(name.to_string());
        }
    }

    /// Ensure that a prim spec exists at `prim`. If it doesn't, create an
    /// `over` spec chain up to the pseudo-root. No-op when the spec is
    /// already present (whether as a `def` or an `over`).
    fn ensure_host_prim(&mut self, prim: &Path) -> Result<()> {
        if self.data.has_spec(prim) {
            return Ok(());
        }
        self.define_over(prim)
    }

    fn register_prop_child(&mut self, prim: &Path, name: &str) {
        self.prop_children
            .entry(prim.clone())
            .or_default()
            .push(name.to_string());
    }

    fn flush_children(&mut self) {
        let prim_children = std::mem::take(&mut self.prim_children);
        for (parent, names) in prim_children {
            if let Some(spec) = self.data.spec_mut(&parent) {
                spec.add(
                    ChildrenKey::PrimChildren,
                    Value::TokenVec(names.into_iter().map(Into::into).collect()),
                );
            }
        }
        let prop_children = std::mem::take(&mut self.prop_children);
        for (prim, names) in prop_children {
            if let Some(spec) = self.data.spec_mut(&prim) {
                spec.add(
                    ChildrenKey::PropertyChildren,
                    Value::TokenVec(names.into_iter().map(Into::into).collect()),
                );
            }
        }
    }
}

/// Build a child prim path from a parent and a simple (unsanitized-caller's
/// responsibility) name.
pub fn child_path(parent: &Path, name: &str) -> Result<Path> {
    let full = if parent.as_str() == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", parent.as_str(), name)
    };
    sdf::path(&full).map_err(anyhow::Error::from)
}

/// Wrap a `Vec<T>` into an `explicit` USD list op (written as `= [...]` in
/// text form with no `prepend` / `append` / `delete` adornment).
fn explicit_list_op<T: Default + Clone + PartialEq>(items: Vec<T>) -> ListOp<T> {
    ListOp {
        explicit: true,
        explicit_items: items,
        ..Default::default()
    }
}
