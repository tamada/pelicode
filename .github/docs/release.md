# Releasing pelicode

English | [日本語](release.ja.md)

A release is made from a branch `release/vX.Y.Z`: the workflows bump the versions on it, and merging it into `main` builds and publishes everything. This page lists what has to be in place, the steps, what to check afterwards, and what to do when something fails.

## What a Release Publishes

| What | Where | Made by |
| --- | --- | --- |
| Git tag `vX.Y.Z` and a GitHub release with notes | [Releases](https://github.com/tamada/pelicode/releases) | `release-start` (shared workflow of tamada/.github) |
| Binaries for Linux, macOS and Windows on x64 and arm64 (`pelicode-X.Y.Z_<os>_<arch>.tar.gz` or `.zip`) | Assets of the release | `publish` job |
| Container image for linux/amd64 and linux/arm64, tagged `X.Y.Z`, `latest` and the short commit | `quay.io/tama5/pelicode` | `docker` and `docker_manifest` jobs |
| The five crates (`pelicode-core`, `-db`, `-detect`, `-crawl`, `-cli`) | crates.io | `publish_to_crates_io` job |
| Homebrew formula | `tamada/tap` | `notify-publish`, through `tamada/api` |

The documentation site is deployed by `deploy documentation` whenever `docs/` changes on `main`, so it follows the merge as well (its version badge is bumped with the rest).

## Prerequisites

Once per repository:

| Secret or setting | Used by | What it is |
| --- | --- | --- |
| `QUAY_ACCOUNT`, `QUAY_TOKEN` | `docker`, `docker_manifest`, `snapshot` | A quay.io robot account with write access to `tama5/pelicode` and `tama5/pelicode-db` |
| `APP_CLIENT_ID`, `APP_PRIVATE_KEY` | `release-finish`, `notify-publish` | The GitHub App `tamada-pipline-bot`, installed for the repository. Publishing the release with its token is what fires `release: published`, which `notify-publish` waits for |
| `SONAR_TOKEN` | `Sonar Qube` | SonarCloud analysis (not needed for the release itself) |
| Coveralls | `build_products` | Enabled for the repository on coveralls.io; uses `GITHUB_TOKEN` |
| GitHub Pages, source "GitHub Actions" | `deploy documentation` | Settings → Pages |
| Trusted publishing on crates.io | `publish_to_crates_io` | For each of the five crates: repository `tamada/pelicode`, workflow `publish.yaml`. See [The First Release](#the-first-release) |
| Public repositories `tama5/pelicode` and `tama5/pelicode-db` on quay.io | images and snapshots | quay.io creates a repository as private on its first push; make it public once |

## Steps

1. **Check `main`.** CI (`build_products`) is green on every platform, and the documentation describes what is released (README, the site, §12 of the specification).

2. **Write the release notes** (optional, recommended): `.github/release-notes/vX.Y.Z.md`, merged into `main` before the release, or added to the release branch. Without them, GitHub generates notes from the pull requests.

3. **Create the release branch** from `main` and push it:

   ```sh
   git switch -c release/vX.Y.Z main
   git push -u origin release/vX.Y.Z
   ```

   `update_version` then commits to the branch "update version to X.Y.Z, ready to publish vX.Y.Z", which sets the version in:
   - `Cargo.toml`: `[workspace.package]` and the versions the crates ask of each other in `[workspace.dependencies]`;
   - `Cargo.lock`;
   - the version badge of `docs/content/_index.md`;
   - `ARG VERSION` of `Containerfile`;
   - `version` and `date-released` of `CITATION.cff`.

4. **Open a pull request** from `release/vX.Y.Z` into `main` after that commit has landed (`git pull` to see it), and wait for CI.

5. **Merge it.** `publish_site_release_assets` runs on the merge:
   1. `notes` finds the release notes, if any;
   2. `setup` tags `main` as `vX.Y.Z` and opens a draft release;
   3. `publish` builds the six binaries and uploads them to the draft;
   4. `docker` builds the image natively on amd64 and arm64, and `docker_manifest` tags the multi-architecture image;
   5. `publish_to_crates_io` publishes the crates in dependency order;
   6. `finish` publishes the release with the token of the App, which starts `notify-publish` and the update of the Homebrew formula.

6. **Check the release** (below).

## After the Release

- The release `vX.Y.Z` is published (not a draft), with six archives that each hold `pelicode` (or `pelicode.exe`), `README.md` and `LICENSE`.
- `pelicode --version` of an archive prints `X.Y.Z`.
- `docker run --rm quay.io/tama5/pelicode:X.Y.Z --version` works on amd64 and arm64.
- `cargo install pelicode` installs `X.Y.Z`.
- `brew upgrade tamada/tap/pelicode` (or `brew install`) installs `X.Y.Z`.
- The version badge of the site shows `X.Y.Z`, and GitHub's "Cite this repository" shows the version and the date.

## The First Release

crates.io can set up trusted publishing only for crates that exist, so the first version of the crates is published by hand with an API token:

1. After step 4 (the version is bumped on the release branch), publish from the head of the release branch:

   ```sh
   git switch release/vX.Y.Z && git pull
   cargo login                      # an API token of crates.io with the publish scopes
   cargo publish --locked --workspace
   ```

2. On crates.io, add the trusted publisher to each of the five crates (Settings → Trusted Publishing: owner `tamada`, repository `pelicode`, workflow `publish.yaml`), and revoke the API token if it is no longer needed.

3. Merge the pull request. `publish_to_crates_io` sees that every crate of the version is on crates.io and skips publishing; later releases publish through trusted publishing.

## When Something Fails

- **A job of `publish_site_release_assets` failed**: re-run the failed jobs from the Actions page; the draft release and the tag stay. `publish` uploads with `--clobber`, so re-running it replaces the assets.
- **The whole run has to be repeated**: run `publish_site_release_assets` by hand (`workflow_dispatch`) with the version, such as `0.1.0`. It works on `main`, so the version bump must be merged already.
- **Only some crates were published**: `publish_to_crates_io` stops, since `cargo publish --workspace` would fail on the published ones. Publish the remaining crates by hand, in dependency order (`cargo publish -p pelicode-db`, ...), then re-run the job.
- **The release stays a draft, or Homebrew is not updated**: `finish` publishes the draft only after every other job succeeded, and `notify-publish` runs only when the release was published with the App token. Check `APP_CLIENT_ID` and `APP_PRIVATE_KEY`, then publish the draft by hand and run `notify-publish` (`workflow_dispatch`).
- **A crate version cannot be taken back**: crates.io only allows yanking (`cargo yank --version X.Y.Z -p <crate>`). Fix forward with a patch release.
