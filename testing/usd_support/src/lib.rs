//! Shared OpenUSD test utilities, synthetic USDA/stage generators, and stage fixtures.

use std::path::{Path, PathBuf};

pub const MODEL_USDA: &[u8] = include_bytes!("fixtures/identity_original.usda");

pub fn synthetic_wide_usda() -> String {
    let mut usda = String::from("#usda 1.0\n\ndef Xform \"World\"\n{\n");
    for group in ["A", "B", "C"] {
        usda.push_str(&format!("    def Xform \"{group}\"\n    {{\n"));
        for i in 0..10 {
            usda.push_str(&format!(
                "        def Xform \"{group}{i}\"\n        {{\n        }}\n"
            ));
        }
        usda.push_str("    }\n");
    }
    usda.push_str("}\n");
    usda
}

pub fn stage_from_usda(usda_text: &str) -> anyhow::Result<openusd::usd::Stage> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = format!("usd_stage_{}_{}.usda", std::process::id(), n);
    let path = std::env::temp_dir().join(file_name);

    {
        let mut file = std::fs::File::create(&path)?;
        file.write_all(usda_text.as_bytes())?;
    }
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("temp path is not valid UTF-8"))?;
    let stage = openusd::usd::Stage::open(path_str);
    let _ = std::fs::remove_file(&path);
    stage
}

pub fn make_synthetic_wide_stage() -> openusd::usd::Stage {
    stage_from_usda(&synthetic_wide_usda()).expect("synthetic wide stage opens")
}

pub fn make_deep_overlap_stage() -> openusd::usd::Stage {
    const DEEP_OVERLAP_USDA: &str = r#"#usda 1.0

def Xform "World"
{
    def Xform "A"
    {
        def Xform "Child"
        {
            def Xform "Leaf"
            {
            }
        }
    }
    def Xform "B"
    {
    }
    def Xform "C"
    {
    }
}
"#;
    stage_from_usda(DEEP_OVERLAP_USDA).expect("deep overlap stage opens")
}

pub fn resolve_fixture_path(relative: &str) -> PathBuf {
    let rel_path = Path::new(relative);
    if rel_path.is_absolute() && rel_path.exists() {
        return rel_path.to_path_buf();
    }
    if rel_path.exists() {
        return rel_path.to_path_buf();
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .ancestors()
        .find(|p| p.join("Cargo.toml").is_file() && p.join("testing").is_dir())
        .unwrap_or_else(|| {
            manifest_dir
                .parent()
                .and_then(Path::parent)
                .unwrap_or(manifest_dir)
        });

    let direct = workspace_root.join(relative);
    if direct.exists() {
        return direct;
    }

    let in_stages = workspace_root.join("tests/stages").join(relative);
    if in_stages.exists() {
        return in_stages;
    }

    let in_tests = workspace_root.join("tests").join(relative);
    if in_tests.exists() {
        return in_tests;
    }

    direct
}

pub fn open_fixture_stage(relative: &str) -> openusd::usd::Stage {
    let path = resolve_fixture_path(relative);
    let path_str = path.to_str().expect("fixture path is valid utf-8");
    openusd::usd::Stage::open(path_str)
        .unwrap_or_else(|err| panic!("failed to open fixture stage at {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_usda_fixture_validity() {
        assert!(!MODEL_USDA.is_empty());
        let stage = stage_from_usda(std::str::from_utf8(MODEL_USDA).unwrap())
            .expect("MODEL_USDA fixture opens as valid Stage");
        assert!(
            stage
                .prim(openusd::sdf::path("/World").unwrap())
                .is_valid()
                .unwrap()
        );
        assert!(
            stage
                .prim(openusd::sdf::path("/World/Building/Wall_A").unwrap())
                .is_valid()
                .unwrap()
        );
    }

    #[test]
    fn test_synthetic_wide_stage_structure() {
        let stage = make_synthetic_wide_stage();
        assert!(
            stage
                .prim(openusd::sdf::path("/World").unwrap())
                .is_valid()
                .unwrap()
        );
        for group in ["A", "B", "C"] {
            let group_path = format!("/World/{group}");
            assert!(
                stage
                    .prim(openusd::sdf::path(&group_path).unwrap())
                    .is_valid()
                    .unwrap()
            );
            for i in 0..10 {
                let prim_path = format!("/World/{group}/{group}{i}");
                assert!(
                    stage
                        .prim(openusd::sdf::path(&prim_path).unwrap())
                        .is_valid()
                        .unwrap()
                );
            }
        }
    }

    #[test]
    fn test_deep_overlap_stage_structure() {
        let stage = make_deep_overlap_stage();
        assert!(
            stage
                .prim(openusd::sdf::path("/World/A/Child/Leaf").unwrap())
                .is_valid()
                .unwrap()
        );
        assert!(
            stage
                .prim(openusd::sdf::path("/World/B").unwrap())
                .is_valid()
                .unwrap()
        );
        assert!(
            stage
                .prim(openusd::sdf::path("/World/C").unwrap())
                .is_valid()
                .unwrap()
        );
    }

    #[test]
    fn test_resolve_fixture_path() {
        let resolved = resolve_fixture_path("empty.usda");
        assert!(
            resolved.exists(),
            "empty.usda fixture should be resolved: {}",
            resolved.display()
        );
    }
}
