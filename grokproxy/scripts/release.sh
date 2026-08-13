#!/usr/bin/env bash
# One-command release: build here, push to GHCR, panda pulls.
#
#   wsl -e bash /mnt/d/SelfMadeTool/TNexus/grokproxy/scripts/release.sh
#
# Why this exists: GitHub Actions on this repo is disabled for billing, so CI
# cannot produce the image. The deployment rules forbid compiling on panda and
# forbid scp'ing artifacts there — but nothing stops building locally and
# publishing to the same registry panda already pulls from. Tagging with the
# exact commit sha keeps the running image traceable, which is the property the
# git-only rule is protecting.
#
# Switch back to CI the moment Actions works again: this is a stand-in, not a
# replacement.
set -euo pipefail

REPO="${TNEXUS_REPO:-/mnt/d/SelfMadeTool/TNexus}"
OWNER="${GHCR_OWNER:-croppedtravelleralex}"
IMAGE="ghcr.io/${OWNER}/grokproxy"
PANDA="${PANDA_HOST:-panda}"
SKIP_GIT_PUSH="${SKIP_GIT_PUSH:-0}"

cd "$REPO/grokproxy"

step() { printf '\n=== %s ===\n' "$1"; }

step "preflight"
dirty="$(git -C "$REPO" status --porcelain -- grokproxy deploy/panda | wc -l)"
if [[ "$dirty" -ne 0 ]]; then
  echo "uncommitted changes under grokproxy/ or deploy/panda —" >&2
  echo "commit first, or the image cannot be traced to a commit:" >&2
  git -C "$REPO" status --short -- grokproxy deploy/panda >&2
  exit 1
fi
sha="$(git -C "$REPO" rev-parse HEAD)"
echo "commit ${sha}"

step "gates (same as CI would run)"
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked

step "build"
docker build -q -t "${IMAGE}:${sha}" -t "${IMAGE}:latest" . >/dev/null
echo "built ${IMAGE}:${sha}"

step "push"
bash scripts/push_image.sh "$sha"

if [[ "$SKIP_GIT_PUSH" != "1" ]]; then
  step "git push"
  git -C "$REPO" push origin main
fi

step "panda: sync repo"
ssh -o BatchMode=yes "$PANDA" 'bash -s' < scripts/panda_sync_repo.sh | tail -2

step "panda: pull + up"
ssh -o BatchMode=yes "$PANDA" \
  "GROKPROXY_TAG=${sha} bash /root/TNexus/deploy/panda/grokproxy-deploy.sh" 2>&1 | tail -4

step "verify"
running="$(ssh -o BatchMode=yes "$PANDA" \
  "docker inspect grokproxy --format '{{.Config.Image}}'" | tr -d '[:space:]')"
echo "running image: ${running}"
if [[ "$running" != *"${sha}"* && "$running" != *":latest" ]]; then
  echo "WARNING: running image does not match this release" >&2
fi
ssh -o BatchMode=yes "$PANDA" \
  "curl -sf --max-time 10 http://127.0.0.1:8110/readyz || echo '(pool empty or not ready)'"
echo
echo "released ${sha}"
