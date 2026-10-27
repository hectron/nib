# Releasing

Prepare a release from anywhere with `gh` signed in:

```bash
gh workflow run prepare-release.yml                    # git-cliff picks the next version
gh workflow run prepare-release.yml -f version=0.3.0   # or choose it
```

The workflow requires `main`, passing CI on its HEAD, and commits since the last tag. It validates that the version is new, updates `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md` on a `release/vX.Y.Z` branch, and opens a release pull request. It then runs CI on the release commit, reports the result as the required `check` status, and enables auto-merge once CI passes — the `main` ruleset still controls the merge.

GitHub holds the release pull request's own `CI` and `Release` runs for approval, because a workflow opened it. That's expected: the reported `check` status replaces them. If the pull request still shows as blocked, approve the held runs:

```bash
gh run list -b release/vX.Y.Z -e pull_request -s action_required --json databaseId -q '.[].databaseId' \
  | xargs -I{} gh api -X POST repos/hectron/nib/actions/runs/{}/approve
```

If `main` moves before the release pull request merges, update the branch with `gh pr update-branch`; running it as yourself starts normal CI.

After the pull request merges, cut the release:

```bash
gh workflow run cut-release.yml
```

The cut workflow reads the version from `Cargo.toml` on `main`, tags that commit, and starts the cargo-dist build. That publishes a GitHub release with tarballs for Apple silicon and Intel, checksums, and a shell installer.

- Release commits are GitHub-verified, and tags are made by `github-actions[bot]` but aren't GPG-signed.
- To test the build without publishing, run `gh workflow run release.yml`; its tag defaults to `dry-run`.
- Homebrew and crates.io publishing come later (see #22).
