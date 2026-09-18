# Default recipe: list available commands
default:
    @just --list

# Build all binaries in release mode
build:
    cargo build --release

# Build in debug mode
build-debug:
    cargo build

# Run all tests
test:
    cargo test

# Run tests for a specific crate
test-crate crate:
    cargo test -p {{ crate }}

# Run clippy lints
lint:
    cargo clippy --all-targets -- -D warnings

# Format code
fmt:
    cargo fmt

# Check formatting without modifying
fmt-check:
    cargo fmt -- --check

# Build and install binaries to the plugin scripts directory
install: build
    cp target/release/session-status plugins/clawde/scripts/
    cp target/release/set-session-name plugins/clawde/scripts/

# Clean build artifacts
clean:
    cargo clean

# Show current version from plugin.json
version:
    @jq -r '.version' plugins/clawde/.claude-plugin/plugin.json

# Bump version across all version files (usage: just bump 2.1.0)
bump new_version:
    #!/usr/bin/env bash
    set -euo pipefail
    old=$(jq -r '.version' plugins/clawde/.claude-plugin/plugin.json)
    echo "Bumping version: ${old} → {{ new_version }}"
    # Plugin JSON files
    jq --arg v "{{ new_version }}" '.version = $v' \
        plugins/clawde/.claude-plugin/plugin.json > /tmp/plugin.json \
        && mv /tmp/plugin.json plugins/clawde/.claude-plugin/plugin.json
    jq --arg v "{{ new_version }}" '.plugins[0].version = $v' \
        .claude-plugin/marketplace.json > /tmp/marketplace.json \
        && mv /tmp/marketplace.json .claude-plugin/marketplace.json
    # Cargo.toml files
    sed -i '' "s/^version = \".*\"/version = \"{{ new_version }}\"/" \
        crates/session-status/Cargo.toml \
        crates/set-session-name/Cargo.toml
    echo "Updated version to {{ new_version }} in:"
    echo "  - plugins/clawde/.claude-plugin/plugin.json"
    echo "  - .claude-plugin/marketplace.json"
    echo "  - crates/session-status/Cargo.toml"
    echo "  - crates/set-session-name/Cargo.toml"

# Full check: fmt, lint, test, build
check: fmt-check lint test build
