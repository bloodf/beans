#!/bin/sh
test -n "$VERCEL_GIT_PREVIOUS_SHA" && git diff --quiet "$VERCEL_GIT_PREVIOUS_SHA" HEAD -- . ../desktop ../package.json ../pnpm-lock.yaml ../pnpm-workspace.yaml ../.npmrc ../packages/beans-blobatar ../crates/models/catalog.json ../crates/cli/marketplace/index.json ../.vercelignore; result=$?; test "$result" -eq 0
