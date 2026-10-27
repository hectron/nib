# Releasing

Releases are cut from `main` by pushing a signed version tag. cargo-dist builds macOS binaries for Apple silicon and Intel, and publishes a GitHub release with tarballs, checksums, and a shell installer. Homebrew and crates.io publishing come later (see issue #22).

1. Pick the version: `git cliff --bumped-version` reads the conventional commits since the last tag.
2. Set `version` in `Cargo.toml`, then run `cargo build` to update `Cargo.lock`.
3. Regenerate the changelog: `git cliff --bump -o CHANGELOG.md`.
4. Commit as `chore(release): vX.Y.Z`, then tag it: `git tag -s vX.Y.Z -m vX.Y.Z`.
5. Push both: `git push origin main vX.Y.Z`. The release workflow does the rest.
