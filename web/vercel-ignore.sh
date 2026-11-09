#!/bin/sh
skip() {
  echo "Skip build: $*"
  exit 0
}

test "${VERCEL_GIT_COMMIT_REF:-}" = main || skip 'only main may deploy automatically.'
test -z "${VERCEL_GIT_PULL_REQUEST_ID:-}" || skip 'pull request deployment.'

# Unknown history does not authorize spending. An operator must review the baseline.
case "$VERCEL_GIT_PREVIOUS_SHA" in
  ''|*[!0123456789abcdefABCDEF]*) skip 'previous SHA missing or invalid; changes unknown, operator review required.' ;;
esac
case "${#VERCEL_GIT_PREVIOUS_SHA}" in
  40|64) ;;
  *) skip 'previous SHA invalid; changes unknown, operator review required.' ;;
esac
test "$(git cat-file -t "$VERCEL_GIT_PREVIOUS_SHA" 2>/dev/null)" = commit || skip 'previous commit unavailable; changes unknown, operator review required.'

# Compare deployed and candidate trees, including all commits in merges and squashes.
git diff --quiet --no-ext-diff --no-renames "$VERCEL_GIT_PREVIOUS_SHA" HEAD -- \
  . ../desktop ../package.json ../pnpm-lock.yaml ../pnpm-workspace.yaml \
  ../.npmrc ../.nvmrc ../.node-version ../.pnpmfile.cjs ../.pnpmfile.mjs \
  ../tsconfig.json ../patches ../packages/beans-blobatar \
  ../crates/models/catalog.json ../crates/cli/marketplace/index.json ../.vercelignore
result=$?
case "$result" in
  0) skip 'site inputs unchanged.' ;;
  1) echo 'Build main: site inputs changed.'; exit 1 ;;
  *) skip 'Git comparison failed; changes unknown, operator review required.' ;;
esac
