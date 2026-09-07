#!/usr/bin/env bash
set -euo pipefail

work_dir=${DEMO_WORK_DIR:-/tmp/things3-demo-gif}
env_file="$work_dir/demo.env"
created_file="$work_dir/created.env"

if [[ -f "$env_file" ]]; then
	# shellcheck source=/dev/null
	source "$env_file"
fi

if [[ -f "$created_file" ]]; then
	# shellcheck source=/dev/null
	source "$created_file"
fi

delete_task_checklists() {
	local task_id=$1
	if [[ -z "$task_id" ]]; then
		return
	fi

	local ids
	ids=$(things3 --no-color project "${HOME_PROJECT:-}" --detailed 2>/dev/null | awk -v task="$task_id" '
    $1 == substr(task, 1, length($1)) { in_task = 1; next }
    in_task && /^[[:space:]]*[A-Za-z0-9]+[[:space:]]+[├└]/ { ids = ids ? ids "," $1 : $1; next }
    in_task && NF && $2 !~ /^[├└]/ { in_task = 0 }
    END { print ids }
  ')

	if [[ -n "$ids" ]]; then
		things3 --no-color edit "$task_id" --remove-checklist "$ids" >/dev/null 2>&1 || true
	fi
}

if [[ -n "${FOLLOW_UP:-}" && -n "${FOLLOW_CHECKS:-}" ]]; then
	things3 --no-color edit "$FOLLOW_UP" --remove-checklist "$FOLLOW_CHECKS" >/dev/null 2>&1 || true
fi

if [[ -n "${BOOK_PLUMBER:-}" ]]; then
	delete_task_checklists "$BOOK_PLUMBER"
fi

for id in \
	"${NEW_TASK:-}" \
	"${RESET:-}" \
	"${REFLECT:-}" \
	"${WEEKLY_SUMMARY:-}" \
	"${REIMBURSEMENT:-}" \
	"${REVIEW_INBOX:-}" \
	"${FOLLOW_UP:-}" \
	"${LAUNCH_BRIEF:-}" \
	"${COMPARE_QUOTES:-}" \
	"${ORDER_FILTERS:-}" \
	"${BOOK_PLUMBER:-}" \
	"${LAUNCH_PROJECT:-}" \
	"${HOME_PROJECT:-}" \
	"${PLANNING_PROJECT:-}"; do
	if [[ -n "$id" ]]; then
		things3 --no-color delete "$id" >/dev/null 2>&1 || true
	fi
done
