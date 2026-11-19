# Dormant projected root tests

This directory contains 37 root-test candidates moved to their mapped quarantine paths under tests/dormant/projection. The repository-local [AUDIT.md](AUDIT.md) is the complete per-file index: it records the original path, mapped destination and responsibility, test names, fixtures and overlap, current API status, related tests, recommendation, and file identity.

## Registration and attachment status

At the C9 audit snapshot (backend baseline HEAD 710cfee36606a4054160150ecdbfa8f171b7a0fa), Cargo.toml sets autotests = false and declares 11 explicit [[test]] targets. None of the 37 candidates is registered, and a static source/test search found no indirect attachment. The quarantine preserved all 45 test functions, zero ignore attributes, and every candidate source blob exactly.

## Compatibility and future handling

As of the C9 API audit, none of these candidates is compatible as written with the current crate API: 32 reference the unavailable UsdAsset symbol, 17 reference the absent usd_schema package, and several use old converter or physics APIs. The candidates were not individually compiled or run.

Keep them dormant as behavior-specific port candidates. Related active tests and shared fixtures can show partial area coverage but do not establish equivalent behavior or obsolescence. Port candidates against the current public API and activate only after a separate Owner-directed review.
