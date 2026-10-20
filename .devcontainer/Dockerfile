# Reproducible contributor environment (#673): the pinned Rust toolchain from
# rust-toolchain.toml, the wasm32-unknown-unknown target, and the Stellar CLI.
FROM mcr.microsoft.com/devcontainers/rust:1-bookworm

# Keep in sync with rust-toolchain.toml.
ARG RUST_TOOLCHAIN=1.94.0
# Prebuilt release binary from https://github.com/stellar/stellar-cli/releases
ARG STELLAR_CLI_VERSION=28.1.0

USER vscode
RUN rustup toolchain install "${RUST_TOOLCHAIN}" \
        --profile minimal \
        --component rust-analyzer,rustfmt,clippy \
        --target wasm32-unknown-unknown \
    && rustup default "${RUST_TOOLCHAIN}"

USER root
RUN set -eux; \
    case "$(uname -m)" in \
        x86_64) target="x86_64-unknown-linux-gnu" ;; \
        aarch64|arm64) target="aarch64-unknown-linux-gnu" ;; \
        *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;; \
    esac; \
    tmp="$(mktemp -d)"; \
    curl -fsSL "https://github.com/stellar/stellar-cli/releases/download/v${STELLAR_CLI_VERSION}/stellar-cli-${STELLAR_CLI_VERSION}-${target}.tar.gz" \
        | tar -xz -C "$tmp"; \
    install -m 0755 "$(find "$tmp" -type f -name stellar | head -n 1)" /usr/local/bin/stellar; \
    rm -rf "$tmp"; \
    stellar --version

USER vscode
