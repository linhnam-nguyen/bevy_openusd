use openusd::sdf::Path;
use openusd::usd::Stage;
use openusd_schemas::geom::{PrimvarsAPI, Subset, SubsetSchema};

use super::util::{
    read_authored_token, read_bool, read_extent, read_float_array, read_int_array,
    read_primvar_interpolation, read_token, read_vec2f_array, read_vec3f_array,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Constant,
    Uniform,
    Varying,
    Vertex,
    FaceVarying,
}

impl Interpolation {
    pub(super) fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "constant" => Self::Constant,
            "uniform" => Self::Uniform,
            "varying" => Self::Varying,
            "vertex" => Self::Vertex,
            "faceVarying" => Self::FaceVarying,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    RightHanded,
    LeftHanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubdivScheme {
    #[default]
    None,
    CatmullClark,
    Loop,
    Bilinear,
}

impl SubdivScheme {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "catmullClark" => Some(Self::CatmullClark),
            "loop" => Some(Self::Loop),
            "bilinear" => Some(Self::Bilinear),
            _ => None,
        }
    }
    pub fn is_subdivision(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone)]
pub struct MeshPrimvar<T> {
    pub values: Vec<T>,
    pub interpolation: Interpolation,
    pub indices: Vec<i32>,
}

#[derive(Debug, Clone)]
pub struct ReadMesh {
    pub points: Vec<[f32; 3]>,
    pub face_vertex_counts: Vec<i32>,
    pub face_vertex_indices: Vec<i32>,
    pub normals: Option<MeshPrimvar<[f32; 3]>>,
    pub uvs: Option<MeshPrimvar<[f32; 2]>>,
    pub orientation: Orientation,
    pub display_color: Option<MeshPrimvar<[f32; 3]>>,
    pub display_opacity: Option<MeshPrimvar<f32>>,
    pub subsets: Vec<ReadSubset>,
    pub double_sided: bool,
    pub extent: Option<[[f32; 3]; 2]>,
    pub subdivision_scheme: SubdivScheme,
}

#[derive(Debug, Clone)]
pub struct ReadSubset {
    pub name: String,
    pub indices: Vec<i32>,
    pub material_binding: Option<Path>,
}

pub fn read_mesh(stage: &Stage, prim: &Path) -> anyhow::Result<Option<ReadMesh>> {
    let Some(points) = read_vec3f_array(stage, prim, "points")? else {
        return Ok(None);
    };
    let Some(face_vertex_counts) = read_int_array(stage, prim, "faceVertexCounts")? else {
        return Ok(None);
    };
    let Some(face_vertex_indices) = read_int_array(stage, prim, "faceVertexIndices")? else {
        return Ok(None);
    };

    let normals = read_primvar_vec3f(stage, prim, "normals")?;
    let uvs = read_primvar_vec2f(stage, prim, "primvars:st")?.or(read_primvar_vec2f(
        stage,
        prim,
        "primvars:st0",
    )?);
    let orientation = match read_token(stage, prim, "orientation")?.as_deref() {
        Some("leftHanded") => Orientation::LeftHanded,
        _ => Orientation::RightHanded,
    };
    let display_color = read_primvar_vec3f(stage, prim, "primvars:displayColor")?;
    let display_opacity = read_primvar_float(stage, prim, "primvars:displayOpacity")?;
    let subsets = read_material_subsets(stage, prim)?;
    let double_sided = read_bool(stage, prim, "doubleSided")?.unwrap_or(false);
    let extent = read_extent(stage, prim)?;
    let subdivision_scheme = read_authored_token(stage, prim, "subdivisionScheme")?
        .as_deref()
        .and_then(SubdivScheme::parse)
        .unwrap_or_default();

    Ok(Some(ReadMesh {
        points,
        face_vertex_counts,
        face_vertex_indices,
        normals,
        uvs,
        orientation,
        display_color,
        display_opacity,
        subsets,
        double_sided,
        extent,
        subdivision_scheme,
    }))
}

/// Read only the authored local extent without decoding mesh topology or
/// primvars. Used by sparse geometry patches that update bounds in place.
pub fn read_mesh_extent(stage: &Stage, prim: &Path) -> anyhow::Result<Option<[[f32; 3]; 2]>> {
    read_extent(stage, prim)
}

fn read_material_subsets(stage: &Stage, mesh_prim: &Path) -> anyhow::Result<Vec<ReadSubset>> {
    let mut out = Vec::new();
    let prim_ref = stage.prim(mesh_prim.clone())?;
    for child_name in prim_ref.child_names()? {
        let Ok(child_path) = mesh_prim.append_path(child_name.as_str()) else {
            continue;
        };
        let child_prim = stage.prim(child_path.clone())?;
        if let Ok(Some(subset)) = Subset::from_prim(child_prim.clone()) {
            if subset.family_name()?.as_ref().map(|t| t.as_str()) != Some("materialBind") {
                continue;
            }
            if matches!(
                subset.element_type()?,
                Some(e) if e != openusd_schemas::geom::ElementType::Face
            ) {
                continue;
            }
            out.push(ReadSubset {
                name: child_name.to_string(),
                indices: subset.indices()?.unwrap_or_default(),
                material_binding: crate::read::shade::read_material_binding(stage, &child_path)?,
            });
            continue;
        }
        if child_prim.type_name()?.as_deref() != Some("GeomSubset") {
            continue;
        }
        if read_token(stage, &child_path, "familyName")?.as_deref() != Some("materialBind") {
            continue;
        }
        if matches!(read_token(stage, &child_path, "elementType")?.as_deref(), Some(e) if e != "face")
        {
            continue;
        }
        out.push(ReadSubset {
            name: child_name.to_string(),
            indices: read_int_array(stage, &child_path, "indices")?.unwrap_or_default(),
            material_binding: crate::read::shade::read_material_binding(stage, &child_path)?,
        });
    }
    Ok(out)
}

fn read_primvar_schema_meta(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> Option<(Option<Interpolation>, Vec<i32>, Option<Path>)> {
    let primvar_name = name.strip_prefix("primvars:")?;
    let prim_ref = stage.prim(prim.clone()).ok()?;
    let api = PrimvarsAPI::from_prim_unchecked(prim_ref);
    let pv = if api.has_primvar(primvar_name).unwrap_or(false) {
        api.primvar(primvar_name).ok()?
    } else {
        api.primvar_with_inheritance(primvar_name).ok()??
    };
    let owner_path = pv.attribute().path().prim_path();
    let interp = pv
        .interpolation()
        .ok()
        .and_then(|i| Interpolation::parse(i.as_token()));
    let indices = pv.indices(None).ok().flatten().unwrap_or_default();
    Some((interp, indices, Some(owner_path)))
}

fn read_primvar_vec3f(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<MeshPrimvar<[f32; 3]>>> {
    let schema_meta = read_primvar_schema_meta(stage, prim, name);
    let source_prim = schema_meta
        .as_ref()
        .and_then(|(_, _, owner)| owner.as_ref())
        .unwrap_or(prim);
    let Some(values) = read_vec3f_array(stage, source_prim, name)? else {
        return Ok(None);
    };
    let interpolation = read_primvar_interpolation(stage, source_prim, name)?
        .or_else(|| schema_meta.as_ref().and_then(|(i, _, _)| *i))
        .unwrap_or(Interpolation::Vertex);
    let indices = match read_int_array(stage, source_prim, &format!("{name}:indices"))? {
        Some(idx) => idx,
        None => schema_meta
            .map(|(_, idx, _)| idx)
            .unwrap_or_default(),
    };
    Ok(Some(MeshPrimvar {
        values,
        interpolation,
        indices,
    }))
}

fn read_primvar_float(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<MeshPrimvar<f32>>> {
    let schema_meta = read_primvar_schema_meta(stage, prim, name);
    let source_prim = schema_meta
        .as_ref()
        .and_then(|(_, _, owner)| owner.as_ref())
        .unwrap_or(prim);
    let Some(values) = read_float_array(stage, source_prim, name)? else {
        return Ok(None);
    };
    let interpolation = read_primvar_interpolation(stage, source_prim, name)?
        .or_else(|| schema_meta.as_ref().and_then(|(i, _, _)| *i))
        .unwrap_or(Interpolation::Vertex);
    let indices = match read_int_array(stage, source_prim, &format!("{name}:indices"))? {
        Some(idx) => idx,
        None => schema_meta
            .map(|(_, idx, _)| idx)
            .unwrap_or_default(),
    };
    Ok(Some(MeshPrimvar {
        values,
        interpolation,
        indices,
    }))
}

fn read_primvar_vec2f(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<MeshPrimvar<[f32; 2]>>> {
    let schema_meta = read_primvar_schema_meta(stage, prim, name);
    let source_prim = schema_meta
        .as_ref()
        .and_then(|(_, _, owner)| owner.as_ref())
        .unwrap_or(prim);
    let Some(values) = read_vec2f_array(stage, source_prim, name)? else {
        return Ok(None);
    };
    let interpolation = read_primvar_interpolation(stage, source_prim, name)?
        .or_else(|| schema_meta.as_ref().and_then(|(i, _, _)| *i))
        .unwrap_or(Interpolation::FaceVarying);
    let indices = match read_int_array(stage, source_prim, &format!("{name}:indices"))? {
        Some(idx) => idx,
        None => schema_meta
            .map(|(_, idx, _)| idx)
            .unwrap_or_default(),
    };
    Ok(Some(MeshPrimvar {
        values,
        interpolation,
        indices,
    }))
}
