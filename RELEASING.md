# Releasing

Run one command from anywhere with `gh` signed in:

```bash
gh workflow run cut-release.yml                    # git-cliff picks the next version
gh workflow run cut-release.yml -f version=0.3.0   # or choose it
```

Or use **Run workflow** on the Cut release workflow in the Actions tab.

The workflow refuses to run unless it's on `main`, CI passed on that commit, and there are commits since the last tag. It then sets the version in `Cargo.toml` and `Cargo.lock`, regenerates `CHANGELOG.md` with git-cliff, commits `chore(release): vX.Y.Z` through GitHub's API (shown as Verified), tags it, and starts the cargo-dist build. That publishes a GitHub release with tarballs for Apple silicon and Intel, checksums, and a shell installer.

Afterward, run `git pull` to pick up the release commit.

- Release commits and tags are made by `github-actions[bot]`; tags aren't GPG-signed.
- To test the build without publishing, run `gh workflow run release.yml`; its tag defaults to `dry-run`.
- Homebrew and crates.io publishing come later (see #22).
