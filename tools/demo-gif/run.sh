#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

if [[ -f "$script_dir/.env" ]]; then
	set -a
	# shellcheck source=/dev/null
	source "$script_dir/.env"
	set +a
fi

if [[ -z "${THINGS3_EMAIL:-}" || -z "${THINGS3_PASSWORD:-}" ]]; then
	printf 'Set THINGS3_EMAIL and THINGS3_PASSWORD for a disposable test account.\n' >&2
	exit 1
fi

work_dir=${DEMO_WORK_DIR:-/tmp/things3-demo-gif}
state_dir=${DEMO_STATE_DIR:-$work_dir/state}

mkdir -p "$work_dir" "$state_dir"

export DEMO_WORK_DIR="$work_dir"
export XDG_STATE_HOME="$state_dir"

"$script_dir/setup.sh"

asciinema rec \
	--overwrite \
	-c "$script_dir/record.sh" \
	"$work_dir/things3-cli-demo.cast"

"$script_dir/render.sh" \
	"$work_dir/things3-cli-demo.cast" \
	"$work_dir/things3-cli-demo.gif"

"$script_dir/cleanup.sh"

printf 'Wrote %s\n' "$work_dir/things3-cli-demo.cast"
printf 'Wrote %s\n' "$work_dir/things3-cli-demo.gif"
