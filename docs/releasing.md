# Releasing

This is the maintainer runbook for cutting an fmeca release. A tag `vX.Y.Z` on
`main` runs [`.github/workflows/release.yml`](../.github/workflows/release.yml),
which builds and publishes the GitHub release, the container image and the MCP
registry entry, and publishes the npm launcher after you approve it. crates.io
is published by hand afterwards.

Throughout, `X.Y.Z` is the new version and `vX.Y.Z` its tag.

## 1. Bump the version

On a branch off `dev`, set the same version in the files that carry it:

- `Cargo.toml`: `[workspace.package] version`.
- `crates/fmeca-mcp/Cargo.toml`: the `version` of the `fmeca` and
  `fmeca-policy` dependencies
  (`fmeca = { path = "../fmeca", version = "X.Y.Z" }`,
  `fmeca-policy = { path = "../fmeca-policy", version = "X.Y.Z" }`).
- `crates/fmeca-policy/Cargo.toml`: the `version` of the `fmeca` dependency
  (`fmeca = { path = "../fmeca", version = "X.Y.Z" }`).
- `Cargo.lock`: run `cargo check` (or
  `cargo update -p fmeca -p fmeca-policy -p fmeca-mcp`) so the lock file
  records the new versions.
- `server.json`: the top-level `version` and the tag of the OCI identifier
  (`ghcr.io/praxec/fmeca:X.Y.Z`). The OCI package has no `version` field: the
  MCP registry rejects one.
- `npm/package.json`: `version` (the npm launcher downloads the release with
  this version, so it must match).

Then check them:

```sh
scripts/check-version-sync.sh
```

It fails when any `server.json` version or OCI tag, the `npm/package.json`
version, an intra-workspace dependency version (`fmeca`/`fmeca-policy` in
`crates/fmeca-mcp/Cargo.toml`, `fmeca` in `crates/fmeca-policy/Cargo.toml`), or
the `fmeca`/`fmeca-policy`/`fmeca-mcp` entries in `Cargo.lock` differ from
`Cargo.toml`. The release workflow repeats the check
against the tag and refuses to build on a mismatch. Nothing else needs a
version bump: the README writes `<version>` for a release tag instead of
pinning one.

## 2. Update the CHANGELOG

In [CHANGELOG.md](../CHANGELOG.md), add a `## [X.Y.Z] - YYYY-MM-DD` section
above the newest release section, listing the changes since that release, and
add its link at the bottom of the file
(`[X.Y.Z]: https://github.com/praxec/fmeca/releases/tag/vX.Y.Z`). If someone
has opened a `## [Unreleased]` section in the meantime, rename it to the new
version instead. Follow Keep a Changelog: group entries under Added, Changed,
Fixed and so on.
The date here is the planned release date; step 3 corrects it on the day.

Open the bump as a pull request into `dev` and merge it once CI is green.

## 3. Release PR from `dev` to `main`

On release day, first set the `## [X.Y.Z] - YYYY-MM-DD` date in
`CHANGELOG.md` to today's date (a one-line pull request into `dev`) if it
differs from the planned one.

Then open a pull request with base `main` and head `dev`. The gitflow guard
(`gitflow-guard.yml`) rejects any other head branch. Wait for every required
check, then merge it with a **merge commit**, not squash or rebase, so `main`
and `dev` keep a shared history.

## 4. Tag `main`

```sh
git fetch origin
git checkout main
git pull --ff-only origin main
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin vX.Y.Z
```

Tag only commits on `main`: the workflow triggers on any `v*` tag. The tag must
match `vMAJOR.MINOR.PATCH` (an optional pre-release or build suffix is
allowed) and equal the `Cargo.toml`, `server.json` and `npm/package.json`
versions.

The workflow then runs these jobs:

1. **validate + draft release**: checks the tag against the versions and
   creates a draft GitHub release with generated notes.
2. **build**: six targets (Linux, macOS and Windows on x86_64 and ARM64). Each
   archive is smoke-tested with `scripts/mcp-smoke.mjs` (MCP `initialize` and
   `tools/list`) and uploaded to the draft.
3. **aggregate metadata + publish**: writes `checksums.sha256` and
   `release-manifest.json` (it fails if the matrix is incomplete), uploads them
   with `install.sh` and `install.ps1`, runs `npm pack` in `npm/` and attaches
   `matthew-cochran-fmeca-X.Y.Z.tgz` with its digest in `npm-package.sha256`,
   and publishes the release.
4. **container image**: builds the image from the repository and pushes
   `ghcr.io/praxec/fmeca` for linux/amd64 and linux/arm64 with the tags
   `X.Y.Z`, `X.Y` and `latest`.
5. **npm publish**: skipped unless npm trusted publishing is set up; the
   maintainer publishes that same tarball with `scripts/npm-release.sh`
   (see step 5).
6. **MCP registry**: publishes `server.json` as `io.github.praxec/fmeca` with
   a pinned, checksum-verified `mcp-publisher` over GitHub OIDC. This job is
   best-effort (`continue-on-error`), so a failure does not fail the release;
   check its log.

## 5. Publish to npm

The npm launcher (`npm/`, package `@matthew-cochran/fmeca`) downloads the
GitHub release whose version equals its own. So publish it only after the
release is public (step 4 ends with the release published). Until then the
documented `npx -y @matthew-cochran/fmeca` and
`npm install -g @matthew-cochran/fmeca` commands fail with a 404 from the
registry.

Publish with the release script, logged in to npm (`npm login`) as an owner of
the `@matthew-cochran` scope and to GitHub (`gh auth login`):

```sh
git checkout vX.Y.Z
bash scripts/npm-release.sh            # version from npm/package.json; or: bash scripts/npm-release.sh X.Y.Z
```

The script:

1. refuses to run unless the GitHub release `vX.Y.Z` exists and is not a draft;
2. downloads the `matthew-cochran-fmeca-X.Y.Z.tgz` that release.yml attached,
   checks it against `npm-package.sha256`, and checks that it holds only
   `LICENSE`, `NOTICE`, `README.md`, `package.json`, `bin/` and `lib/`;
3. publishes that file with `scripts/npm-publish.sh --local`, so npm prompts
   for your 2FA one-time password. A version already on npm is skipped, and
   pre-releases (`X.Y.Z-rc.N`) go to the `next` dist-tag;
4. waits (up to `FMECA_RELEASE_WAIT_SECS`, default 180) for the registry to
   serve the version, then checks that
   `npx -y @matthew-cochran/fmeca@X.Y.Z --version` prints `fmeca-mcp X.Y.Z`.

It never tags, pushes or edits the release. The same approach as jevitate's
`scripts/release.sh`: no npm token is stored, and the publish carries no
provenance attestation (only a trusted-publishing run can create one).

### Optional: npm trusted publishing

release.yml has an **npm publish** job that publishes the same tarball over
GitHub OIDC with provenance. It is off until you set it up:

1. On npmjs.com, the package's **Settings → Trusted publishing** names GitHub
   Actions, the repository `praxec/fmeca`, the workflow `release.yml` and the
   environment `npm`.
2. On GitHub, the `npm` environment has required reviewers and allows only
   `v*` tags.
3. Set the repository variable `NPM_TRUSTED_PUBLISHING` to `true`.

The job then waits for approval in the `npm` environment after each release;
approving it is the whole npm step. It skips a version already on npm, so it is
safe after a manual publish.

**The npm trusted publisher must name the workflow file exactly `release.yml` (cpm-planner 0.2.2's publish 404'd because it said `release.yaml`).**

## 6. Verify the release

Assets: the release page lists six archives, `checksums.sha256`,
`release-manifest.json`, `install.sh`, `install.ps1`,
`matthew-cochran-fmeca-X.Y.Z.tgz` and `npm-package.sha256`.

```sh
gh release view vX.Y.Z --json assets --jq '.assets[].name'
gh release download vX.Y.Z --pattern checksums.sha256 --pattern '*linux-gnu.tar.gz'
sha256sum -c --ignore-missing checksums.sha256
```

Installer, on a clean shell (repeat with `install.ps1` on Windows):

```sh
curl -fsSL https://github.com/praxec/fmeca/releases/latest/download/install.sh | sh -s -- --version vX.Y.Z
node scripts/mcp-smoke.mjs ~/.local/bin/fmeca-mcp
```

Container image:

```sh
docker pull ghcr.io/praxec/fmeca:X.Y.Z
node scripts/mcp-smoke.mjs docker run -i --rm ghcr.io/praxec/fmeca:X.Y.Z
```

MCP registry: confirm the registry job succeeded, or look the server up in the
registry and check that its version is `X.Y.Z`.

## 7. Publish to crates.io (manual)

The workflow does not publish the crates. From the tagged commit, with a
crates.io token configured (`cargo login`), publish in dependency order:
`fmeca` first, then `fmeca-policy`, then `fmeca-mcp`, because `fmeca-policy`
and `fmeca-mcp` depend on `fmeca` and `fmeca-mcp` depends on `fmeca-policy`:

```sh
git checkout vX.Y.Z
cargo publish -p fmeca --dry-run --locked
cargo package -p fmeca --list --locked   # no docs/superpowers, .github or scripts
cargo publish -p fmeca --locked

# Wait for `fmeca X.Y.Z` to appear on crates.io, then publish fmeca-policy.
cargo publish -p fmeca-policy --dry-run --locked
cargo publish -p fmeca-policy --locked
```

Wait for `fmeca-policy X.Y.Z` to appear on crates.io, then publish `fmeca-mcp`.
Paste this block whole: it runs in a subshell that stops at the first failure,
so nothing is published when the working tree has changes besides the copied
skills (`--allow-dirty` must cover only those):

```sh
(
  set -euo pipefail
  # skills/ is copied in as a generated packaging input (gitignored); see build.rs
  rm -rf crates/fmeca-mcp/skills && cp -R skills crates/fmeca-mcp/skills
  dirty="$(git status --porcelain --untracked-files=all | grep -v ' crates/fmeca-mcp/skills/' || true)"
  if [ -n "$dirty" ]; then
    printf '%s\n' "$dirty"
    echo "the working tree has changes besides crates/fmeca-mcp/skills/; not publishing" >&2
    exit 1
  fi
  cargo publish -p fmeca-mcp --dry-run --locked --allow-dirty
  cargo publish --locked -p fmeca-mcp --allow-dirty
  rm -rf crates/fmeca-mcp/skills
)
```

Until this step runs, crates.io keeps serving the previous versions, which is
why the README installs from the git tag. docs.rs builds the new versions on
its own a few minutes after publishing.

CI checks this step on every pull request: the `package` job runs
`cargo publish --workspace --dry-run --locked --allow-dirty` (with the skills
copied in), which packages and verifies all three crates against each other in
publish order, so it passes even before a version bump reaches crates.io.

## Rollback

- **A build failed before publishing.** The release is still a draft. Fix the
  problem on `dev`, then delete the draft and the tag and re-release with the
  next patch version (or re-push the same tag if nothing was published):

  ```sh
  gh release delete vX.Y.Z --yes
  git push origin :refs/tags/vX.Y.Z
  git tag -d vX.Y.Z
  ```

  Re-running the workflow on an existing draft is also safe: assets are
  uploaded with `--clobber`, and an existing release is left as is.
- **A bad release was published.** Do not reuse its version. Mark the GitHub
  release as a pre-release or edit its notes to warn users, then ship a fixed
  `X.Y.(Z+1)`. The installers resolve `releases/latest`, so they pick up the
  fix as soon as it is published.
- **A bad crate was published.** crates.io versions cannot be deleted or
  overwritten. Yank it so new lock files do not pick it up, then publish a fix:

  ```sh
  cargo yank --version X.Y.Z fmeca
  # undo: cargo yank --version X.Y.Z --undo fmeca
  ```

- **The image job failed or timed out.** The GitHub release and npm are not
  affected. Re-run the release workflow, or rebuild the image locally and push
  the same tags.
- **A bad image was pushed.** Push the fixed release, which moves `latest` and
  `X.Y`. Delete the bad `X.Y.Z` tag from the GHCR package settings only if it is
  harmful to keep.
- **A bad npm version was published.** npm versions cannot be reused.
  Deprecate it and publish the fixed version:

  ```sh
  npm deprecate @matthew-cochran/fmeca@X.Y.Z "broken release, use X.Y.(Z+1)"
  ```

  `npm unpublish` works only within 72 hours and blocks the version number
  for good, so prefer deprecation.
- **A bad MCP registry entry.** Publish the fixed version; the registry lists
  the newest version.
