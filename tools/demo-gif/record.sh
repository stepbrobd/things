#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${TERM:-}" || "${TERM:-}" == "dumb" ]]; then
	export TERM=xterm-256color
fi

work_dir=${DEMO_WORK_DIR:-/tmp/things3-demo-gif}
# shellcheck source=/dev/null
source "$work_dir/demo.env"

created="$work_dir/created.env"
: >"$created"

short_id() {
	printf '%s' "${1:0:2}"
}

HOME_PROJECT_SHORT=$(short_id "$HOME_PROJECT")
REVIEW_INBOX_SHORT=$(short_id "$REVIEW_INBOX")
BOOK_PLUMBER_SHORT=$(short_id "$BOOK_PLUMBER")
FIRST_PLUMBER_CHECK_SHORT=$(short_id "$FIRST_PLUMBER_CHECK")
SECOND_PLUMBER_CHECK_SHORT=$(short_id "$SECOND_PLUMBER_CHECK")

pause() {
	sleep "${1:-1.6}"
}

prompt() {
	printf '\n\033[38;5;8m$\033[0m '
	type_command "$@"
	printf '\n'
}

quote_arg() {
	local arg=$1
	local tick='`'

	if [[ "$arg" =~ ^[A-Za-z0-9_./:=@%+,-]+$ ]]; then
		printf '%s' "$arg"
		return
	fi

	arg=${arg//\\/\\\\}
	arg=${arg//"/\\"/}
	arg=${arg//\$/\\\$}
	arg=${arg//$tick/\\$tick}
	printf '"%s"' "$arg"
}

format_command() {
	local first=1
	local arg

	for arg in "$@"; do
		if ((first)); then
			first=0
		else
			printf ' '
		fi

		quote_arg "$arg"
	done
}

type_command() {
	local command
	command=$(format_command "$@")

	if [[ "${DEMO_TYPE_COMMANDS:-1}" == "0" ]]; then
		printf '%s' "$command"
		return
	fi

	local i
	local delay=${DEMO_TYPE_DELAY:-0.035}
	for ((i = 0; i < ${#command}; i++)); do
		printf '%s' "${command:i:1}"
		sleep "$delay"
	done
}

run() {
	prompt "$@"
	"$@"
	pause
}

run_read() {
	prompt "$@"
	"$@"
	pause 3.0
}

run_quick() {
	prompt "$@"
	"$@"
	pause 1.2
}

run_capture() {
	prompt "$@" >&2
	local output
	output=$("$@" 2>&1)
	printf '%s\n' "$output" >&2
	pause

	printf '%s\n' "$output" |
		perl -pe 's/\e\[[0-9;]*m//g' |
		tr -d '\r' |
		grep -oE '[A-Za-z0-9]{22}' |
		tail -1
}

run_clear() {
	prompt clear
	sleep 0.4
	printf '\033[2J\033[H'
}

run_read things3 today
pause 1.2

run_quick things3 mark "$REVIEW_INBOX_SHORT" --done

new_task=$(run_capture things3 new "Call dentist about night guard" --when today --notes "Ask whether the scan from last year is still usable.")
printf 'NEW_TASK=%q\n' "$new_task" >>"$created"
pause 0.8

run_clear
run_read things3 today --detailed
pause 0.8

run_clear

run_read things3 projects list
run_quick things3 schedule "$HOME_PROJECT_SHORT" --when today
run_read things3 project "$HOME_PROJECT_SHORT"

run_quick things3 edit "$BOOK_PLUMBER_SHORT" --add-checklist "send building availability"
plain_project=$(things3 --no-color project "$HOME_PROJECT" --detailed)
new_check=$(printf '%s\n' "$plain_project" | awk '/send building availability/ { print $1; exit }')
printf 'NEW_PLUMBER_CHECK=%q\n' "$new_check" >>"$created"

run_read things3 project "$HOME_PROJECT_SHORT" --detailed
pause 1.0

run_quick things3 mark "$BOOK_PLUMBER_SHORT" --check "$FIRST_PLUMBER_CHECK_SHORT"
run_quick things3 mark "$BOOK_PLUMBER_SHORT" --check "$SECOND_PLUMBER_CHECK_SHORT"
run_read things3 project "$HOME_PROJECT_SHORT" --detailed
