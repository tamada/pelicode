#!/usr/bin/env bash
# Runs `pelicode scan` for the GitHub Action: writes SARIF, the number of
# functions with candidates to $GITHUB_OUTPUT, and a summary of the job.
#
#   PELICODE                      the pelicode binary
#   DATABASE, CONFIG, ARGS        options given to the action
#   RANGE, PATHS                  what to check; derived from the event when both are empty
#   SARIF_FILE                    where to write the SARIF log
#   EVENT_NAME, BASE_SHA, HEAD_SHA, BEFORE, AFTER  from the event
set -euo pipefail

range=${RANGE:-}
paths=${PATHS:-}
if [[ -z "$range" && -z "$paths" ]]; then
  case "$EVENT_NAME" in
    pull_request | pull_request_target) range="$BASE_SHA...$HEAD_SHA" ;;
    push)
      # A new branch has no previous commit to compare with.
      if [[ -n "${BEFORE:-}" && ! "$BEFORE" =~ ^0+$ ]]; then
        range="$BEFORE..$AFTER"
      else
        paths=.
      fi
      ;;
    *) paths=. ;;
  esac
fi

command=("$PELICODE" scan)
if [[ -n "$range" ]]; then
  command+=(--git "$range")
else
  read -ra targets <<< "$paths"
  command+=("${targets[@]}")
fi
[[ -n "${DATABASE:-}" ]] && command+=(--db "$DATABASE")
[[ -n "${CONFIG:-}" ]] && command+=(--config "$CONFIG")
if [[ -n "${ARGS:-}" ]]; then
  read -ra extra <<< "$ARGS"
  command+=("${extra[@]}")
fi
command+=(-f sarif -o "$SARIF_FILE")

echo "Running: ${command[*]}"
rm -f "$SARIF_FILE"
status=0
"${command[@]}" || status=$?
# Status 1 with a SARIF log means candidates were found with fail_on_match in
# the settings; the action decides on failing after the upload instead.
if [[ $status -ne 0 ]] && ! { [[ $status -eq 1 ]] && jq -e . "$SARIF_FILE" > /dev/null 2>&1; }; then
  exit "$status"
fi

count=$(jq '[.runs[].results[]] | length' "$SARIF_FILE")
{
  echo "candidates=$count"
  echo "sarif-file=$SARIF_FILE"
} >> "$GITHUB_OUTPUT"

{
  echo "## pelicode"
  echo
  if [[ -n "$range" ]]; then
    echo "Checked the functions changed in \`$range\`."
  else
    echo "Checked \`$paths\`."
  fi
  echo
  if [[ "$count" -eq 0 ]]; then
    echo "No similar code candidates found."
  else
    echo "$count function(s) have similar code candidates to review. pelicode does not decide whether code was copied; check each candidate and its license."
    echo
    jq -r '.runs[].results[]
      | "- `\(.locations[0].physicalLocation.artifactLocation.uri):\(.locations[0].physicalLocation.region.startLine)`: \(.message.text)"' \
      "$SARIF_FILE"
  fi
} >> "${GITHUB_STEP_SUMMARY:-/dev/null}"
