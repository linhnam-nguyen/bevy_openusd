//! Thin Rust integration wrapper invoking the single canonical Python guard
//! (`scripts/check_test_architecture.py`) and its self-test suite.

use std::path::Path;
use std::process::{Command, Output};

fn find_python_311() -> (String, Vec<String>) {
    let mut candidates: Vec<(String, Vec<String>)> = Vec::new();
    for env_key in ["USDHUB_PYTHON", "PYTHON"] {
        if let Ok(value) = std::env::var(env_key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                candidates.push((trimmed.to_string(), Vec::new()));
            }
        }
    }
    candidates.push(("python3".to_string(), Vec::new()));
    candidates.push(("python".to_string(), Vec::new()));
    candidates.push(("py".to_string(), vec!["-3".to_string()]));

    let version_probe = "import sys; raise SystemExit(0 if sys.version_info >= (3, 11) else 1)";
    for (exe, prefix) in candidates {
        let mut cmd = Command::new(&exe);
        cmd.args(&prefix).args(["-B", "-c", version_probe]);
        if let Ok(status) = cmd.status()
            && status.success()
        {
            return (exe, prefix);
        }
    }

    panic!(
        "Python >= 3.11 is required to execute scripts/check_test_architecture.py \
         (set USDHUB_PYTHON or ensure python3/python/py -3 >= 3.11 is on PATH)"
    );
}

fn run_python(manifest_dir: &Path, args: &[&str]) -> Output {
    let (exe, prefix) = find_python_311();
    let mut cmd = Command::new(&exe);
    cmd.current_dir(manifest_dir)
        .args(&prefix)
        .arg("-B")
        .args(args);
    cmd.output()
        .unwrap_or_else(|err| panic!("failed to spawn `{exe}` with args {args:?}: {err}"))
}

#[test]
fn workspace_satisfies_canonical_test_architecture_guard() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = run_python(
        manifest_dir,
        &[
            "scripts/check_test_architecture.py",
            "--root",
            ".",
            "--json",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "canonical test architecture guard failed (status={:?})\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("invalid guard JSON: {err}\nstdout:\n{stdout}"));
    assert_eq!(report["status"].as_str(), Some("pass"));
    assert_eq!(report["cargo_metadata_used"].as_bool(), Some(true));
    assert_eq!(report["violation_count"].as_u64(), Some(0));
    for key in ["test_module_count", "tests_root_count"] {
        assert!(
            report[key].as_u64().is_some_and(|count| count >= 1),
            "expected canonical {key} >= 1 in guard JSON output:\n{stdout}"
        );
    }
}

#[test]
fn canonical_test_architecture_guard_self_tests_pass() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = run_python(
        manifest_dir,
        &[
            "-m",
            "unittest",
            "scripts/test_check_test_architecture.py",
            "-v",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "canonical guard self-test suite failed (status={:?})\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
}
