# Releasing

This is the maintainer runbook for cutting an fmeca release. A tag `vX.Y.Z` on
`main` runs [`.github/workflows/release.yml`](../.github/workflows/release.yml),
which builds and publishes the GitHub release, the container image and the MCP
registry entry, and publishes the npm launcher after you approve it. crates.io
is published by hand afterwards.

Throughout, `X.Y.Z` is the new version and `vX.Y.Z` its tag.

## 1. Bump the version

On a branch off `dev`, set the same version in the five files that carry it:

- `Cargo.toml`: `[workspace.package] version`.
- `crates/fmeca-mcp/Cargo.toml`: the `version` of the `fmeca` dependency
  (`fmeca = { path = "../fmeca", version = "X.Y.Z" }`).
- `Cargo.lock`: run `cargo check` (or `cargo update -p fmeca -p fmeca-mcp`) so
  the lock file records the new version.
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
version, the `fmeca` dependency version in `crates/fmeca-mcp/Cargo.toml`, or
the `fmeca`/`fmeca-mcp` entries in `Cargo.lock` differ from `Cargo.toml`. The release workflow repeats the check
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
5. **npm publish**: waits for approval in the `npm` environment, then
   publishes that same tarball as `@matthew-cochran/fmeca` (see step 5).
6. **MCP registry**: publishes `server.json` as `io.github.praxec/fmeca` with
   a pinned, checksum-verified `mcp-publisher` over GitHub OIDC. This job is
   best-effort (`continue-on-error`), so a failure does not fail the release;
   check its log.

## 5. Approve the npm publish

The `npm publish` job publishes the npm launcher (`npm/`, package
`@matthew-cochran/fmeca`) with npm trusted publishing: npm authenticates the
job over GitHub OIDC, so the repository holds no npm token, and the package
gets a provenance attestation. The job runs in the `npm` environment, which
waits for a maintainer's approval. Approve it from the run page (**Review
deployments**) as soon as the release is public (for 0.1.0, first follow
[First release only](#first-release-only-010) below). The launcher downloads the
GitHub release whose version equals its own, and until the publish the
documented `npx -y @matthew-cochran/fmeca` and
`npm install -g @matthew-cochran/fmeca` commands fail with a 404 from the
registry.

The job publishes the tarball attached to the release after checking it against
`npm-package.sha256`, so npm gets exactly that file. A version that is already
on npm is skipped, so re-running the job is safe. Pre-release versions
(`X.Y.Z-rc.N`) go to the `next` dist-tag. The job ends by running
`npx -y @matthew-cochran/fmeca@X.Y.Z --version`.

One-time setup: on npmjs.com, the package's
**Settings → Trusted publishing** names GitHub Actions, the repository
`praxec/fmeca`, the workflow `release.yml` and the environment `npm`; on
GitHub, the `npm` environment has required reviewers and allows only `v*` tags.

**The npm trusted publisher must name the workflow file exactly `release.yml` (cpm-planner 0.2.2's publish 404'd because it said `release.yaml`).**

### First release only (0.1.0)

npm can only set up trusted publishing for a package that already exists, so
0.1.0 is published by hand, from the tarball the release attached, before the
`npm` job is approved:

1. Tag `v0.1.0` (step 4).
2. The **aggregate metadata + publish** job attaches
   `matthew-cochran-fmeca-0.1.0.tgz` and `npm-package.sha256` to the release.
   The **npm publish** job then waits for approval in the `npm` environment;
   do not approve it yet.
3. Download both files and check the digest:

   ```sh
   mkdir -p /tmp/fmeca-npm && cd /tmp/fmeca-npm
   gh release download v0.1.0 --repo praxec/fmeca \
     --pattern 'matthew-cochran-fmeca-0.1.0.tgz' --pattern npm-package.sha256
   sha256sum -c npm-package.sha256          # must print OK; stop if it does not
   ```

4. Logged in to npm (`npm login`) as an owner of the `@matthew-cochran` scope,
   publish that tarball with 2FA:

   ```sh
   npm publish ./matthew-cochran-fmeca-0.1.0.tgz --access public   # prompts for the 2FA code
   ```

5. On npmjs.com, configure the trusted publisher for
   `@matthew-cochran/fmeca`: GitHub Actions, repository `praxec/fmeca`,
   workflow `release.yml`, environment `npm`.
6. Approve the **npm publish** job. It skips the publish, because 0.1.0 is
   already on npm, and runs the `npx -y @matthew-cochran/fmeca@0.1.0 --version`
   verification.

From 0.1.1 on, approving the job is the whole npm step.

### Manual fallback

If the job cannot publish (for example, trusted publishing is misconfigured),
publish the same tarball by hand. You need to be logged in to npm
(`npm login`) as an owner of the `@matthew-cochran` scope, with 2FA:

```sh
mkdir -p /tmp/fmeca-npm && cd /tmp/fmeca-npm
gh release download vX.Y.Z --repo praxec/fmeca \
  --pattern 'matthew-cochran-fmeca-X.Y.Z.tgz' --pattern npm-package.sha256
sha256sum -c npm-package.sha256          # must print OK; stop if it does not
tar -tzf matthew-cochran-fmeca-X.Y.Z.tgz # package/{LICENSE,NOTICE,README.md,package.json,bin/,lib/} only, no tests
npm publish ./matthew-cochran-fmeca-X.Y.Z.tgz --access public   # prompts for the 2FA code
```

Verify from a clean cache, outside the repository:

```sh
npx -y @matthew-cochran/fmeca@X.Y.Z --version
node scripts/mcp-smoke.mjs npx -y @matthew-cochran/fmeca@X.Y.Z   # from a checkout of vX.Y.Z
```

The first command prints `fmeca-mcp X.Y.Z` on stdout after a one-time
download whose progress goes to stderr. The smoke test checks that the launcher
keeps stdout clean for MCP (`initialize` and `tools/list`).

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
crates.io token configured (`cargo login`), publish the kernel first and the
MCP server second, because `fmeca-mcp` depends on `fmeca`:

```sh
git checkout vX.Y.Z
cargo publish -p fmeca --dry-run --locked
cargo package -p fmeca --list --locked   # no docs/superpowers, .github or scripts
cargo publish -p fmeca --locked
# Wait for `fmeca X.Y.Z` to appear on crates.io, then:
# skills/ is copied in as a generated packaging input (gitignored); see build.rs
rm -rf crates/fmeca-mcp/skills && cp -R skills crates/fmeca-mcp/skills
# --allow-dirty must only cover the copied skills: stop if anything else shows.
if git status --porcelain --untracked-files=all | grep -v ' crates/fmeca-mcp/skills/'; then
  echo "the working tree has other changes; stop before publishing"; false
fi
cargo publish -p fmeca-mcp --dry-run --locked --allow-dirty
cargo publish --locked -p fmeca-mcp --allow-dirty
rm -rf crates/fmeca-mcp/skills
```

Until this step runs, crates.io keeps serving the previous versions, which is
why the README installs from the git tag.

After the first release (once `fmeca` 0.1.0 is on crates.io), restore the full
`fmeca-mcp` publish dry-run in CI: in the `package` job of
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml), replace
`cargo package --locked --list -p fmeca-mcp --allow-dirty` with
`cargo publish --dry-run --locked -p fmeca-mcp --allow-dirty` and drop the
comment that explains the workaround. Open it as a pull request into `dev`. docs.rs builds the new versions on
its own a few minutes after publishing.

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
