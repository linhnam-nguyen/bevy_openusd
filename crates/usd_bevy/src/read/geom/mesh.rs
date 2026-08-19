use openusd::sdf::Path;
use openusd::usd::Stage;

use super::util::{
    read_bool, read_extent, read_float_array, read_int_array, read_primvar_interpolation,
    read_token, read_vec2f_array, read_vec3f_array,
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
    let subdivision_scheme = read_token(stage, prim, "subdivisionScheme")?
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

fn read_material_subsets(stage: &Stage, mesh_prim: &Path) -> anyhow::Result<Vec<ReadSubset>> {
    let mut out = Vec::new();
    for child_name in stage.prim(mesh_prim.clone()).child_names()? {
        let Ok(child_path) = mesh_prim.append_path(child_name.as_str()) else {
            continue;
        };
        if stage.prim(child_path.clone()).type_name()?.as_deref() != Some("GeomSubset") {
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

fn read_primvar_vec3f(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<MeshPrimvar<[f32; 3]>>> {
    let Some(values) = read_vec3f_array(stage, prim, name)? else {
        return Ok(None);
    };
    Ok(Some(MeshPrimvar {
        values,
        interpolation: read_primvar_interpolation(stage, prim, name)?
            .unwrap_or(Interpolation::Vertex),
        indices: read_int_array(stage, prim, &format!("{name}:indices"))?.unwrap_or_default(),
    }))
}

fn read_primvar_float(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<MeshPrimvar<f32>>> {
    let Some(values) = read_float_array(stage, prim, name)? else {
        return Ok(None);
    };
    Ok(Some(MeshPrimvar {
        values,
        interpolation: read_primvar_interpolation(stage, prim, name)?
            .unwrap_or(Interpolation::Vertex),
        indices: read_int_array(stage, prim, &format!("{name}:indices"))?.unwrap_or_default(),
    }))
}

fn read_primvar_vec2f(
    stage: &Stage,
    prim: &Path,
    name: &str,
) -> anyhow::Result<Option<MeshPrimvar<[f32; 2]>>> {
    let Some(values) = read_vec2f_array(stage, prim, name)? else {
        return Ok(None);
    };
    Ok(Some(MeshPrimvar {
        values,
        interpolation: read_primvar_interpolation(stage, prim, name)?
            .unwrap_or(Interpolation::FaceVarying),
        indices: read_int_array(stage, prim, &format!("{name}:indices"))?.unwrap_or_default(),
    }))
}
