#!/bin/bash
set -euo pipefail

# Script to delete all GitHub deployments except the latest one

REPO="RefractSystems/virtmcu"

if [ -z "${GH_TOKEN:-}" ]; then
  echo "Error: GH_TOKEN is not set."
  exit 1
fi

echo "Fetching deployments for $REPO..."
# Fetch all deployments, sorted by created_at (descending)
DEPLOYMENTS=$(gh api "repos/$REPO/deployments" --paginate --jq 'sort_by(.created_at) | reverse | .[].id')

if [ -z "$DEPLOYMENTS" ]; then
  echo "No deployments found."
  exit 0
fi

# Convert to array
# shellcheck disable=SC2206
DEP_ARRAY=($DEPLOYMENTS)
COUNT=${#DEP_ARRAY[@]}

echo "Found $COUNT deployments."

if [ "$COUNT" -le 1 ]; then
  echo "Only one or zero deployments exist. Nothing to delete."
  exit 0
fi

LATEST_ID=${DEP_ARRAY[0]}
echo "Keeping latest deployment: $LATEST_ID"

# Delete all others
for (( i=1; i<$COUNT; i++ )); do
  DEP_ID=${DEP_ARRAY[$i]}
  echo "Deleting old deployment: $DEP_ID..."
  gh api -X DELETE "repos/$REPO/deployments/$DEP_ID" --silent
done

echo "Done."
