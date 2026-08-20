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

/// Undo/redo stack over the authoring ops.
#[derive(Default)]
pub struct EditHistory {
    undo: Vec<(Op, Op)>, // (forward, inverse)
    redo: Vec<(Op, Op)>,
}

impl EditHistory {
    fn record(&mut self, stage: &Stage, forward: Op, inverse: Op) -> Result<()> {
        forward.apply(stage)?;
        self.undo.push((forward, inverse));
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
        let old = stage
            .prim(openusd::sdf::path(prim)?)
            .attribute(name)
            .get::<Value>()
            .ok()
            .flatten();
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
        let Some((fwd, inv)) = self.undo.pop() else {
            return Ok(false);
        };
        inv.apply(stage)?;
        self.redo.push((fwd, inv));
        Ok(true)
    }

    pub fn redo(&mut self, stage: &Stage) -> Result<bool> {
        let Some((fwd, inv)) = self.redo.pop() else {
            return Ok(false);
        };
        fwd.apply(stage)?;
        self.undo.push((fwd, inv));
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
