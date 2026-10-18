[private]
@default: help

git_revision := `git rev-parse --short HEAD`
app_version := `awk -F'"' '/^\[workspace.package\]/{p=1} p && /^version *=/{print $2; exit}' Cargo.toml`
build_date := `date -u +%Y-%m-%dT%H:%M:%SZ`

# show help message
@help:
    echo "Task runner for pelicode {{ app_version }} with Just"
    echo "Usage: just <recipe>"
    echo ""
    just --list

# Build the project with cargo build
build target = "": format clippy
    cargo build --locked --workspace {{ target }}

# Run the tests and measure the coverage (target/coverage.lcov, sent to Coveralls by CI)
test: (build "")
    cargo llvm-cov --locked --workspace --lcov --output-path target/coverage.lcov

# Only ubuntu reports the coverage, and llvm-profdata cannot read the profiles of Windows on arm64.
# Run the tests without measuring the coverage.
test_without_coverage: (build "")
    cargo test --locked --workspace

# Check that the source codes are formatted, without rewriting them.
verify_format:
    cargo fmt --all --check

# Run cargo fmt for formatting the source codes.
format:
    cargo fmt --all

# Run clippy for checking the source codes.
clippy:
    cargo clippy --locked --workspace --all-targets -- -D warnings

# Build the API documentation, failing on warnings such as broken links.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --document-private-items

# Check that every crate can be packaged for crates.io, without publishing.
publish_check:
    cargo publish --locked --workspace --dry-run

# Build the documentation site into docs/public with Hugo
site:
    cd docs && hugo --minify

# Start the Hugo server for the documentation site, at http://localhost:1313/pelicode/
start:
    cd docs && hugo server

# Build the docker image of pelicode
docker:
    docker build \
        --build-arg GIT_REVISION={{git_revision}} \
        --build-arg BUILD_DATE={{build_date}} \
        --build-arg VERSION={{app_version}} \
        -f Containerfile \
        -t quay.io/tama5/pelicode:latest \
        -t quay.io/tama5/pelicode:{{app_version}} \
        .

# Build the docker image for multiple platforms and push them into quay.io
docker_buildx:
    docker buildx build \
        --build-arg GIT_REVISION={{git_revision}} \
        --build-arg BUILD_DATE={{build_date}} \
        --build-arg VERSION={{app_version}} \
        -f Containerfile \
        --platform linux/arm64/v8,linux/amd64 \
        --output=type=image,push=true \
        -t quay.io/tama5/pelicode:latest \
        -t quay.io/tama5/pelicode:{{app_version}} \
        .

snapshot_repository := "quay.io/tama5/pelicode-db"
snapshot_media_type := "application/vnd.pelicode.snapshot.v1.sqlite+zstd"

# Compress a snapshot and push it into quay.io as NAME-YYYYMMDD and NAME-latest (needs zstd, oras)
snapshot_push file name:
    zstd -19 -T0 --force "{{file}}" -o "{{file}}.zst"
    cd "$(dirname "{{file}}")" && oras push \
        --artifact-type application/vnd.pelicode.snapshot.v1 \
        "{{snapshot_repository}}:{{name}}-$(date -u +%Y%m%d),{{name}}-latest" \
        "$(basename "{{file}}").zst:{{snapshot_media_type}}"

# Pull a snapshot from quay.io (TAG such as pypi-latest) and decompress it into FILE
snapshot_pull tag file:
    tmp="$(mktemp -d)" && oras pull "{{snapshot_repository}}:{{tag}}" -o "$tmp" \
        && zstd -d --force "$tmp"/*.zst -o "{{file}}" && rm -r "$tmp"
