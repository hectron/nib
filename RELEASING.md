# Releasing

Prepare a release from anywhere with `gh` signed in:

```bash
gh workflow run prepare-release.yml                    # git-cliff picks the next version
gh workflow run prepare-release.yml -f version=0.3.0   # or choose it
```

The workflow requires `main`, passing CI on its HEAD, and commits since the last tag. It validates that the version is new, updates `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md` on a `release/vX.Y.Z` branch, and opens a release pull request. It then starts CI and enables auto-merge — the `main` ruleset still controls the merge.

After the pull request merges, cut the release:

```bash
gh workflow run cut-release.yml
```

The cut workflow reads the version from `Cargo.toml` on `main`, tags that commit, and starts the cargo-dist build. That publishes a GitHub release with tarballs for Apple silicon and Intel, checksums, and a shell installer.

- Release commits are GitHub-verified, and tags are made by `github-actions[bot]` but aren't GPG-signed.
- To test the build without publishing, run `gh workflow run release.yml`; its tag defaults to `dry-run`.
- Homebrew and crates.io publishing come later (see #22).
