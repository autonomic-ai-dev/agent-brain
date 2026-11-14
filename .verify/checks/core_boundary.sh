#!/usr/bin/env bash
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DIR/../.." && pwd)"

FORBIDDEN='^(rusqlite|tokio|rmcp|async-nats|ureq|keyring|libc|fastembed|opendal|tonic|prost|age|secrecy|tar|zstd|walkdir|dirs|tracing|anyhow|thiserror|chrono|uuid|sha2|lru) v'

cd "$REPO_ROOT"
if cargo tree -p tekmor-core -e normal --prefix none | grep -E "$FORBIDDEN"; then
  echo "core boundary broken: tekmor-core depends on storage, runtime, or IO crates" >&2
  exit 1
fi
echo "core boundary holds: tekmor-core has no storage, runtime, or IO dependencies"
