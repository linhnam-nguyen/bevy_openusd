use openusd::sdf::Value;
use openusd::usd::{NamespaceEditor, Stage};

type Result<T> = anyhow::Result<T>;

// ─── Namespace ops ──────────────────────────────────────────────────

/// Define (create) a prim of `type_name` at `path`.
pub fn define_prim(stage: &Stage, path: &str, type_name: &str) -> Result<()> {
    let prim = openusd::sdf::path(path)?;
    stage.define_prim(prim)?.set_type_name(type_name)?;
    Ok(())
}

/// Remove the prim (and its subtree) at `path`. Returns whether anything was
/// removed.
pub fn remove_prim(stage: &Stage, path: &str) -> Result<bool> {
    let prim = openusd::sdf::path(path)?;
    Ok(stage.remove_prim(prim)?)
}

/// Rename the prim at `path` to `new_name` (last namespace component only).
pub fn rename_prim(stage: &Stage, path: &str, new_name: &str) -> Result<()> {
    let prim = stage.prim(openusd::sdf::path(path)?);
    let mut editor = NamespaceEditor::new(stage);
    editor.rename_prim(&prim, new_name)?;
    editor.apply()?;
    Ok(())
}

/// Reparent the prim at `path` under `new_parent` (keeping its name).
pub fn reparent_prim(stage: &Stage, path: &str, new_parent: &str) -> Result<()> {
    let prim = stage.prim(openusd::sdf::path(path)?);
    let parent = stage.prim(openusd::sdf::path(new_parent)?);
    let mut editor = NamespaceEditor::new(stage);
    editor.reparent_prim(&prim, &parent)?;
    editor.apply()?;
    Ok(())
}

/// Move the prim at `old` to the absolute path `new` (rename + reparent in one).
pub fn move_prim(stage: &Stage, old: &str, new: &str) -> Result<()> {
    let mut editor = NamespaceEditor::new(stage);
    editor.move_prim(openusd::sdf::path(old)?, openusd::sdf::path(new)?);
    editor.apply()?;
    Ok(())
}

// ─── Attribute ops ──────────────────────────────────────────────────

/// Author `value` onto attribute `name` of `prim` (creating it as
/// `type_name` if needed).
pub fn set_attribute(
    stage: &Stage,
    prim: &str,
    name: &str,
    type_name: &str,
    value: Value,
) -> Result<()> {
    let attr = openusd::sdf::path(prim)?.append_property(name)?;
    stage.create_attribute(attr, type_name)?.set(value)?;
    Ok(())
}

/// Clear an authored attribute opinion (in the current edit target).
pub fn clear_attribute(stage: &Stage, prim: &str, name: &str) -> Result<bool> {
    let attr = openusd::sdf::path(prim)?.append_property(name)?;
    Ok(stage.remove_property(attr)?)
}

// ─── Variant selection (PLAN Phase 2) ───────────────────────────────

/// Select variant `selection` for variant set `set` on `prim` (non-destructive:
/// other sets' selections are preserved). This authors the prim's
/// `variantSelection` metadata — a **composition** change, so the commit fires a
/// `resynced` notice and the live loop reconciles the affected subtree.
pub fn set_variant(stage: &Stage, prim: &str, set: &str, selection: &str) -> Result<()> {
    let set = set.to_string();
    let selection = selection.to_string();
    stage
        .prim(openusd::sdf::path(prim)?)
        .update_metadata("variantSelection", move |cur| {
            let mut map = match cur {
                Some(Value::VariantSelectionMap(m)) => m,
                _ => std::collections::HashMap::new(),
            };
            map.insert(set, selection);
            Value::VariantSelectionMap(map)
        })?;
    Ok(())
}

/// The prim's currently authored/composed selection for `set`, if any.
pub(super) fn current_variant(stage: &Stage, prim: &str, set: &str) -> Option<String> {
    openusd::sdf::path(prim)
        .ok()
        .and_then(|p| crate::read::variants::variant_selection(stage, &p, set))
}

// ─── Persistence (P6) ───────────────────────────────────────────────

/// Serialize the stage's composed root layer to a `.usda` string.
pub fn export_stage_string(stage: &Stage) -> Result<String> {
    stage.root_layer().export_to_string()
}

/// Write the stage's root layer to `filename` (a `.usda`/`.usd` path).
pub fn save_stage_as(stage: &Stage, filename: &str) -> Result<()> {
    stage.root_layer().export(filename)?;
    Ok(())
}

/// Whether a prim with an authored type currently resolves on the stage.
pub fn prim_exists(stage: &Stage, path: &str) -> bool {
    openusd::sdf::path(path)
        .ok()
        .map(|p| {
            stage
                .prim(p)
                .type_name()
                .map(|t| t.is_some())
                .unwrap_or(false)
        })
        .unwrap_or(false)
}
