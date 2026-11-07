#!/bin/sh
case "$VERCEL_GIT_PREVIOUS_SHA" in
  ''|*[!0123456789abcdefABCDEF]*) exit 1 ;;
esac
case "${#VERCEL_GIT_PREVIOUS_SHA}" in
  40|64) ;;
  *) exit 1 ;;
esac
test "$(git cat-file -t "$VERCEL_GIT_PREVIOUS_SHA")" = commit || exit 1
git diff --quiet "$VERCEL_GIT_PREVIOUS_SHA" HEAD -- . ../desktop ../package.json ../pnpm-lock.yaml ../pnpm-workspace.yaml ../.npmrc ../packages/beans-blobatar ../crates/models/catalog.json ../crates/cli/marketplace/index.json ../.vercelignore; result=$?; test "$result" -eq 0
