#!/usr/bin/env bash
# Fast-forward /root/TNexus when untracked local copies shadow now-tracked files.
# Nothing is deleted: conflicting files are moved into a timestamped backup so
# the merge can proceed and the originals stay recoverable.
set -euo pipefail

ROOT="${TNEXUS_ROOT:-/root/TNexus}"
cd "$ROOT"

git fetch origin main >/dev/null 2>&1

# Files that exist untracked here but are tracked at origin/main would be
# clobbered by the merge; git refuses rather than overwrite them.
mapfile -t blockers < <(
  git merge --ff-only origin/main 2>&1 \
    | sed -n 's/^\t//p' \
    | grep -v '^$' || true
)

if [[ ${#blockers[@]} -gt 0 ]]; then
  backup="$ROOT/.merge-backup-$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$backup"
  echo "moving ${#blockers[@]} conflicting file(s) to $backup"
  for file in "${blockers[@]}"; do
    [[ -e "$file" ]] || continue
    mkdir -p "$backup/$(dirname "$file")"
    mv "$file" "$backup/$file"
    echo "  $file"
  done
fi

git merge --ff-only origin/main
echo "now at: $(git log --oneline -1)"
