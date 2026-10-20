# Scripts

Development utility scripts.

| Script | Description |
|--------|-------------|
| `setup-hooks.sh` | Install Git pre-commit hooks |
| `pre-commit-check.sh` | Run pre-commit checks manually |
| `generate-openapi.sh` | Regenerate OpenAPI spec from Rust code |
| `buf-gen.sh` | Generate protobuf code from `.proto` files |
| `buf-export.sh` | Export protobuf schemas |
| `retire-1.0.0-artifacts.sh` | After `0.1.0-rc.1` is verified: dry-run / `--execute` delete of premature `1.0.0*` git tags and the container images and Helm charts published for them |
| `osrb-oss-inventory.py` | Regenerate the third-party OSS inventory from `Cargo.lock` for nSpect/OSRB, classifying each component against the OSRB license allowlist — see [`docs/oss-compliance.md`](../docs/oss-compliance.md) |

## Pre-commit Checks

The pre-commit hook runs:
- YAML validation
- `cargo fmt` (auto-fixes formatting)
- `cargo clippy` (Rust linting)
- `helm lint` (with required `app.thumbnailSizes` override)
- Unit tests

Bypass with `git commit --no-verify` (not recommended).
