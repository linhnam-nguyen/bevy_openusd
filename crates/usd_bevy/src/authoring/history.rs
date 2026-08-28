use openusd::sdf::Value;
use openusd::usd::Stage;

use super::ops::{
    clear_attribute, current_variant, define_prim, remove_prim, rename_prim, reparent_prim,
    set_attribute, set_variant,
};

type Result<T> = anyhow::Result<T>;

// ─── Undo / redo over authoring ops (RETHINK §13) ───────────────────

/// `(parent, name)` split of an absolute prim path.
fn split_path(p: &str) -> (&str, &str) {
    match p.rfind('/') {
        Some(0) => ("/", &p[1..]),
        Some(i) => (&p[..i], &p[i + 1..]),
        None => ("", p),
    }
}

#[derive(Clone)]
enum Op {
    Define {
        path: String,
        type_name: String,
    },
    Remove {
        path: String,
    },
    SetAttr {
        prim: String,
        name: String,
        type_name: String,
        value: Option<Value>,
    },
    RenameTo {
        path: String,
        new_name: String,
    },
    ReparentTo {
        path: String,
        new_parent: String,
    },
    SetVariant {
        prim: String,
        set: String,
        selection: Option<String>,
    },
}

/// One authored attribute in a grouped edit.
///
/// The value is already encoded for the target USD type. Keeping this small
/// adapter in `usd_bevy` lets callers validate a complete batch before the
/// stage writer starts committing any member of it.
#[derive(Clone)]
pub struct AttributeEdit {
    pub prim: String,
    pub name: String,
    pub type_name: String,
    pub value: Value,
}

impl Op {
    fn apply(&self, stage: &Stage) -> Result<()> {
        match self {
            Op::Define { path, type_name } => define_prim(stage, path, type_name),
            Op::Remove { path } => remove_prim(stage, path).map(|_| ()),
            Op::SetAttr {
                prim,
                name,
                type_name,
                value,
            } => match value {
                Some(v) => set_attribute(stage, prim, name, type_name, v.clone()),
                None => clear_attribute(stage, prim, name).map(|_| ()),
            },
            Op::RenameTo { path, new_name } => rename_prim(stage, path, new_name),
            Op::ReparentTo { path, new_parent } => reparent_prim(stage, path, new_parent),
            Op::SetVariant {
                prim,
                set,
                selection,
            } => match selection {
                Some(sel) => set_variant(stage, prim, set, sel),
                // Restoring "no selection" clears the set back to its default.
                None => set_variant(stage, prim, set, ""),
            },
        }
    }
}

#[derive(Clone)]
struct EditGroup {
    forward: Vec<Op>,
    inverse: Vec<Op>,
}

/// Undo/redo stack over the authoring ops.
#[derive(Default)]
pub struct EditHistory {
    undo: Vec<EditGroup>,
    redo: Vec<EditGroup>,
}

impl EditHistory {
    fn record(&mut self, stage: &Stage, forward: Op, inverse: Op) -> Result<()> {
        self.record_group(
            stage,
            EditGroup {
                forward: vec![forward],
                inverse: vec![inverse],
            },
        )
    }

    fn record_group(&mut self, stage: &Stage, group: EditGroup) -> Result<()> {
        let mut applied = 0;
        for operation in &group.forward {
            if let Err(error) = operation.apply(stage) {
                for inverse in group.inverse[..applied].iter().rev() {
                    let _ = inverse.apply(stage);
                }
                return Err(error);
            }
            applied += 1;
        }
        self.undo.push(group);
        self.redo.clear();
        Ok(())
    }

    pub fn define(&mut self, stage: &Stage, path: &str, type_name: &str) -> Result<()> {
        let fwd = Op::Define {
            path: path.into(),
            type_name: type_name.into(),
        };
        let inv = Op::Remove { path: path.into() };
        self.record(stage, fwd, inv)
    }

    pub fn set_attr(
        &mut self,
        stage: &Stage,
        prim: &str,
        name: &str,
        type_name: &str,
        value: Value,
    ) -> Result<()> {
        let old = current_attribute_value(stage, prim, name)?;
        let fwd = Op::SetAttr {
            prim: prim.into(),
            name: name.into(),
            type_name: type_name.into(),
            value: Some(value),
        };
        let inv = Op::SetAttr {
            prim: prim.into(),
            name: name.into(),
            type_name: type_name.into(),
            value: old,
        };
        self.record(stage, fwd, inv)
    }

    /// Applies all attribute edits as one stage transaction from the
    /// editor's undo/redo perspective. Every old value is captured before
    /// the first write, and a failed forward operation rolls back already
    /// applied members without publishing a partial history entry.
    pub fn set_attrs_atomic(&mut self, stage: &Stage, edits: &[AttributeEdit]) -> Result<()> {
        if edits.is_empty() {
            anyhow::bail!("atomic attribute edit batch is empty");
        }

        let mut forward = Vec::with_capacity(edits.len());
        let mut inverse = Vec::with_capacity(edits.len());
        for edit in edits {
            let old = current_attribute_value(stage, &edit.prim, &edit.name)?;
            forward.push(Op::SetAttr {
                prim: edit.prim.clone(),
                name: edit.name.clone(),
                type_name: edit.type_name.clone(),
                value: Some(edit.value.clone()),
            });
            inverse.push(Op::SetAttr {
                prim: edit.prim.clone(),
                name: edit.name.clone(),
                type_name: edit.type_name.clone(),
                value: old,
            });
        }

        self.record_group(stage, EditGroup { forward, inverse })
    }

    pub fn rename(&mut self, stage: &Stage, path: &str, new_name: &str) -> Result<()> {
        let (parent, old_name) = split_path(path);
        let new_path = if parent == "/" {
            format!("/{new_name}")
        } else {
            format!("{parent}/{new_name}")
        };
        let fwd = Op::RenameTo {
            path: path.into(),
            new_name: new_name.into(),
        };
        let inv = Op::RenameTo {
            path: new_path,
            new_name: old_name.into(),
        };
        self.record(stage, fwd, inv)
    }

    pub fn reparent(&mut self, stage: &Stage, path: &str, new_parent: &str) -> Result<()> {
        let (old_parent, name) = split_path(path);
        let new_path = if new_parent == "/" {
            format!("/{name}")
        } else {
            format!("{new_parent}/{name}")
        };
        let fwd = Op::ReparentTo {
            path: path.into(),
            new_parent: new_parent.into(),
        };
        let inv = Op::ReparentTo {
            path: new_path,
            new_parent: old_parent.into(),
        };
        self.record(stage, fwd, inv)
    }

    /// Select `selection` for variant `set` on `prim`, recording the prior
    /// selection for undo.
    pub fn set_variant(
        &mut self,
        stage: &Stage,
        prim: &str,
        set: &str,
        selection: &str,
    ) -> Result<()> {
        let old = current_variant(stage, prim, set);
        let fwd = Op::SetVariant {
            prim: prim.into(),
            set: set.into(),
            selection: Some(selection.into()),
        };
        let inv = Op::SetVariant {
            prim: prim.into(),
            set: set.into(),
            selection: old,
        };
        self.record(stage, fwd, inv)
    }

    pub fn undo(&mut self, stage: &Stage) -> Result<bool> {
        let Some(group) = self.undo.pop() else {
            return Ok(false);
        };
        let mut applied: Vec<usize> = Vec::with_capacity(group.inverse.len());
        for index in (0..group.inverse.len()).rev() {
            if let Err(error) = group.inverse[index].apply(stage) {
                for applied_index in applied.iter().rev() {
                    let _ = group.forward[*applied_index].apply(stage);
                }
                self.undo.push(group);
                return Err(error);
            }
            applied.push(index);
        }
        self.redo.push(group);
        Ok(true)
    }

    pub fn redo(&mut self, stage: &Stage) -> Result<bool> {
        let Some(group) = self.redo.pop() else {
            return Ok(false);
        };
        let mut applied = 0;
        for operation in &group.forward {
            if let Err(error) = operation.apply(stage) {
                for inverse in group.inverse[..applied].iter().rev() {
                    let _ = inverse.apply(stage);
                }
                self.redo.push(group);
                return Err(error);
            }
            applied += 1;
        }
        self.undo.push(group);
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

fn current_attribute_value(stage: &Stage, prim: &str, name: &str) -> Result<Option<Value>> {
    Ok(stage
        .prim(openusd::sdf::path(prim)?)
        .attribute(name)
        .get::<Value>()?)
}
