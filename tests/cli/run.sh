#!/usr/bin/env bash
set -euo pipefail

export TZ=UTC
export NO_COLOR=1
export THINGS_LOG="off,things_cli::cloud_commit::request=debug"
export THINGS_LOG_FORMAT=json

# the runner behind every trycmd case
# things gets --no-cloud and a fixed --today-ts unless the case sets them, and --load-journal journal.json when the case directory holds one

# the auth file and the sync cache of every run live in a scratch directory
# a case without a journal folds that empty cache, never the one of whoever runs the tests
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
export XDG_CONFIG_HOME="$scratch/config"
export XDG_STATE_HOME="$scratch/state"

argv=("$@")

if [[ ${#argv[@]} -gt 0 && "${argv[0]}" == "things" ]]; then
	if [[ -n "${TRYCMD_BIN_THINGS:-}" ]]; then
		argv[0]="${TRYCMD_BIN_THINGS}"
	fi

	has_no_cloud=0
	has_load_journal=0
	has_today_ts=0
	for ((i = 1; i < ${#argv[@]}; i++)); do
		if [[ "${argv[i]}" == "--no-cloud" ]]; then
			has_no_cloud=1
		elif [[ "${argv[i]}" == "--load-journal" ]]; then
			has_load_journal=1
		elif [[ "${argv[i]}" == "--today-ts" ]]; then
			has_today_ts=1
		fi
	done

	globals=()
	if [[ $has_no_cloud -eq 0 ]]; then
		globals+=("--no-cloud")
	fi
	if [[ $has_today_ts -eq 0 ]]; then
		globals+=("--today-ts" "1774396800")
	fi

	if [[ $has_load_journal -eq 0 && -f "journal.json" ]]; then
		globals+=("--load-journal" "journal.json")
	fi

	if [[ ${#globals[@]} -gt 0 ]]; then
		argv=("${argv[0]}" "${globals[@]}" "${argv[@]:1}")
	fi
fi

stderr_file="$scratch/stderr"

set +e
"${argv[@]}" 2>"$stderr_file"
status=$?
set -e

# commit request log events become their pretty printed payload, every other stderr line passes through in place
jq -RrS '(fromjson? | objects | select(.event == "cloud.commit.request") | .request_json | fromjson) // .' <"$stderr_file" 1>&2

exit "$status"
