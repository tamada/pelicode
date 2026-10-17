# Container image of the pelicode command.
#
#   docker run --rm -v "$PWD":/workdir quay.io/tama5/pelicode scan .
#
# The working directory is /workdir, so the database (pelicode.db by default)
# and the code to check are read from and written to the mounted directory.

FROM rust:1-slim-trixie AS builder

WORKDIR /src
COPY . .

# The C code of the Tree-sitter grammars and of the bundled SQLite needs a C compiler,
# which the slim image carries.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p pelicode-cli \
    && cp target/release/pelicode /usr/local/bin/pelicode

FROM debian:trixie-slim

ARG GIT_REVISION=unknown
ARG BUILD_DATE=unknown
ARG VERSION=unknown

LABEL org.opencontainers.image.title="pelicode" \
      org.opencontainers.image.description="Know where your code came from." \
      org.opencontainers.image.source="https://github.com/tamada/pelicode" \
      org.opencontainers.image.url="https://tamada.github.io/pelicode" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.authors="Haruaki Tamada" \
      org.opencontainers.image.revision="${GIT_REVISION}" \
      org.opencontainers.image.created="${BUILD_DATE}" \
      org.opencontainers.image.version="${VERSION}"

# git for `pelicode scan --git`, CA certificates for `pelicode crawl`.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 pelicode \
    && mkdir /workdir \
    && chown pelicode:pelicode /workdir \
    # The mounted repository belongs to another user; let git read it anyway.
    && git config --system --add safe.directory '*'

COPY --from=builder /usr/local/bin/pelicode /usr/local/bin/pelicode

USER pelicode
WORKDIR /workdir

ENTRYPOINT ["pelicode"]
CMD ["--help"]
