# Projects Owner Review 5 C0 baseline

This records the zero-behavior baseline for Owner Review 5 after Owner Review
4 C1-C7+ was accepted and frozen in the authoritative implementation plan.

```text
backend branch: develop/project-peerView
backend SHA before C0: c0d237bee68e98261ac92670ac66ad4c68609d64
UI branch: projects-peerView
UI SHA before C0: 8a2f207a1492f3b1a4f8771c8017ea13847271b3
project manifest schema: 1
viewport protocol: 5
project write protocol: 3
project activation protocol: 3
project read protocol: 1
project scene inspection protocol: 1
project model preparation protocol: 1
project import progress protocol: 1
project cache descriptor schema: 2
scene/model authored wrapper schema: 1
```

The frozen OR4 product boundary remains the baseline: Project composition is
rooted at the protected Root Scene; Project/Scene/Model identities and stable
USD prim paths remain authoritative; Import Scene is USD-only; unsupported
non-USD Model import is unavailable; UI hierarchy state is derived from the
active Stage generation; Commit and Export are not yet implemented.

Baseline validation:

```text
cargo fmt --all -- --check: passed
git diff --check: passed
cargo check --workspace: passed; existing warnings only
cargo test --workspace: 147 passed, 1 known unrelated M19 failure
./scripts/check_rust_file_size.sh: passed; 613 files, 56 warnings, 0 failures
python3 -B scripts/check_performance_regressions.py: passed
```

The known test failure is the pre-existing
`src/project/service/m19_tests.rs:102:60` `DirectoryNotEmpty` cleanup race.
The repository also retains the previously recorded all-feature environment
limits (`DLSS_SDK` and Vulkan). No browser, GPU, native Tauri, or production
evidence is claimed by C0.

OR5 C0 introduces no product behavior. Subsequent checkpoints must preserve
the OR4 freeze and update the implementation plan with their exact commit and
remote evidence.
