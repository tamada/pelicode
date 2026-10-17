#!/usr/bin/env bash
# Installs pelicode for the GitHub Action and writes its path to $GITHUB_OUTPUT.
#
#   PELICODE_PATH     an installed binary to use, if any
#   PELICODE_VERSION  a release such as 0.1.0, or "source"
#   ACTION_REF        the ref the action is used at; a tag vX.Y.Z selects that release
#   GITHUB_ACTION_PATH, RUNNER_OS, RUNNER_ARCH, RUNNER_TEMP, GH_TOKEN  as set by GitHub
set -euo pipefail

repository=tamada/pelicode

if [[ -n "${PELICODE_PATH:-}" ]]; then
  bin=$(cd "$(dirname "$PELICODE_PATH")" && pwd)/$(basename "$PELICODE_PATH")
else
  version=${PELICODE_VERSION:-}
  if [[ -z "$version" && "${ACTION_REF:-}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    version=$ACTION_REF
  fi
  if [[ -z "$version" || "$version" == "source" ]]; then
    echo "Building pelicode from the source of the action"
    cargo install --locked --quiet --root "$RUNNER_TEMP/pelicode" --path "$GITHUB_ACTION_PATH/cli"
    bin=$RUNNER_TEMP/pelicode/bin/pelicode
  else
    version=${version#v}
    case "$RUNNER_OS-$RUNNER_ARCH" in
      Linux-X64) platform=linux_amd64 ;;
      Linux-ARM64) platform=linux_arm64 ;;
      macOS-X64) platform=darwin_amd64 ;;
      macOS-ARM64) platform=darwin_arm64 ;;
      Windows-X64) platform=windows_amd64 ;;
      Windows-ARM64) platform=windows_arm64 ;;
      *) echo "::error::no pelicode release for $RUNNER_OS on $RUNNER_ARCH" >&2; exit 1 ;;
    esac
    archive=pelicode-${version}_${platform}
    echo "Downloading $archive of $repository v$version"
    gh release download "v$version" --repo "$repository" --pattern "$archive.*" --dir "$RUNNER_TEMP" --clobber
    if [[ "$RUNNER_OS" == Windows ]]; then
      (cd "$RUNNER_TEMP" && unzip -oq "$archive.zip")
      bin=$RUNNER_TEMP/pelicode-$version/pelicode.exe
    else
      tar xzf "$RUNNER_TEMP/$archive.tar.gz" -C "$RUNNER_TEMP"
      bin=$RUNNER_TEMP/pelicode-$version/pelicode
    fi
  fi
fi

"$bin" --version
echo "path=$bin" >> "$GITHUB_OUTPUT"
