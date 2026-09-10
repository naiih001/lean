# Releasing lean

## Ritual (60s)

1. **Bump version** in `Cargo.toml` (and let `Cargo.lock` update):
   ```bash
   # example: 0.2.0 → 0.3.0
   # edit Cargo.toml version, then:
   cargo check
   ```

2. **Update `CHANGELOG.md`**: Move `## [Unreleased]` to `## [0.3.0] - YYYY-MM-DD` with notes, and recreate empty `## [Unreleased]` on top.

3. **Commit and push**:
   ```bash
   git add Cargo.toml Cargo.lock CHANGELOG.md
   git commit -m "chore: release v0.3.0"
   git push origin main
   ```

4. **Tag and push tag** (triggers `release.yml`):
   ```bash
   git tag v0.3.0
   git push origin v0.3.0
   ```

5. **Watch** `Actions → release` workflow — it builds `linux-x86_64`, `macos-x86_64`, `windows-x86_64`, packages `tar.gz`/`zip` + `.sha256`, and publishes GitHub Release with auto-notes.

6. **Verify installers** (after Release is green):
   ```bash
   # unix dry-run via pinned version
   LEAN_VERSION=v0.3.0 bash -n install.sh && echo ok
   curl -fsSL https://raw.githubusercontent.com/naiih001/lean/main/install.sh | bash  # or with LEAN_VERSION
   # windows (PowerShell on Windows)
   # irm https://raw.githubusercontent.com/naiih001/lean/main/install.ps1 | iex
   ```

7. **(Optional)** `cargo publish` if you want crates.io.

## Workflow details

- Trigger: `push` tag `v*.*.*` or `workflow_dispatch`.
- Jobs: `build` matrix (ubuntu/macos/windows) → `cargo build --release --target ...` → `tar.gz`/`zip` + `sha256` → `release` job merges artifacts and calls `softprops/action-gh-release@v2` with `generate_release_notes: true` and install snippet.
- Required secret: default `GITHUB_TOKEN` (no extra setup).
- Artifacts: `lean-linux-x86_64.tar.gz` (+`.sha256`), `lean-macos-x86_64.tar.gz`, `lean-windows-x86_64.zip`.

## Preflight checklist

- `cargo test` green, `cargo check` no errors
- `bash -n install.sh` ok, `install.ps1` syntax ok
- `README.md` install matrix reflects current assets
- `CHANGELOG.md` has entry for version

## Troubleshooting

- **Workflow not triggered**: ensure tag is `vX.Y.Z` (lowercase v) and pushed to `origin`.
- **Linux build fails libssl**: `release.yml` installs `libssl-dev`; if cache invalidates, rerun.
- **macOS x86_64 on arm64 runner**: target is `x86_64-apple-darwin` cross — works but binary runs via Rosetta on Apple Silicon; add `aarch64` target later if needed.

## Post-release

- `git tag --list | head`, `gh release view v0.3.0 --json assets` (if `gh` installed)
- Update `install.sh`/`install.ps1` docs if assets change
- File follow-up issue for `rustls` / musl static if Linux portability becomes pain point
