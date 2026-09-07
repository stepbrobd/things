#!/usr/bin/env bash
set -euo pipefail

work_dir=${DEMO_WORK_DIR:-/tmp/things3-demo-gif}
mkdir -p "$work_dir"

out="$work_dir/demo.env"
: >"$out"

capture_id() {
	local output
	if ! output=$("$@" 2>&1); then
		printf '%s\n' "$output" >&2
		return 1
	fi

	printf '%s\n' "$output" | tr -d '\r' | awk 'END {print $NF}'
}

write_env() {
	local key=$1
	local value=$2
	printf '%s=%q\n' "$key" "$value" >>"$out"
}

deadline=$(date -d '+7 days' +%F)

planning_project=$(capture_id things3 --no-color projects new "Weekly Planning" --when today)
home_project=$(capture_id things3 --no-color projects new "Apartment Repairs" --when anytime)
launch_project=$(capture_id things3 --no-color projects new "Website Refresh" --when anytime)

follow_up=$(capture_id things3 --no-color new "Send API review notes to Alex" --when today --notes "Summarize the auth edge cases from yesterday's call.")
review_inbox=$(capture_id things3 --no-color new "Triage Things inbox" --when today)
reimbursement=$(capture_id things3 --no-color new "Submit March reimbursement" --when today --deadline "$deadline")
weekly_summary=$(capture_id things3 --no-color new "Prepare weekly planning notes" --in "$planning_project" --when today)
reflect=$(capture_id things3 --no-color new "Pick tomorrow's top three" --when today)
things3 --no-color schedule "$reflect" --when evening >/dev/null
reset=$(capture_id things3 --no-color new "Clear desk before bed" --when today)
things3 --no-color schedule "$reset" --when evening >/dev/null

book_plumber=$(capture_id things3 --no-color new "Schedule kitchen sink repair" --in "$home_project" --notes "Drain is slow after running the disposal.")
order_filters=$(capture_id things3 --no-color new "Order Aarke filter refills" --in "$home_project")
compare_quotes=$(capture_id things3 --no-color new "Check cabinet hinge measurements" --in "$home_project")
launch_brief=$(capture_id things3 --no-color new "Rewrite homepage intro" --in "$launch_project")

things3 --no-color edit "$follow_up" \
	--add-checklist "pull Slack thread" \
	--add-checklist "send summary" >/dev/null

things3 --no-color edit "$book_plumber" \
	--add-checklist "check building calendar" \
	--add-checklist "text super for preferred vendor" >/dev/null

plain_today=$(things3 --no-color today --detailed)
follow_checks=$(printf '%s\n' "$plain_today" | awk '/pull Slack thread|send summary/ { ids = ids ? ids "," $1 : $1 } END { print ids }')

plain_project=$(things3 --no-color project "$home_project" --detailed)
first_plumber_check=$(printf '%s\n' "$plain_project" | awk '/check building calendar/ { print $1; exit }')
second_plumber_check=$(printf '%s\n' "$plain_project" | awk '/text super for preferred vendor/ { print $1; exit }')

write_env PLANNING_PROJECT "$planning_project"
write_env HOME_PROJECT "$home_project"
write_env LAUNCH_PROJECT "$launch_project"
write_env FOLLOW_UP "$follow_up"
write_env REVIEW_INBOX "$review_inbox"
write_env REIMBURSEMENT "$reimbursement"
write_env WEEKLY_SUMMARY "$weekly_summary"
write_env REFLECT "$reflect"
write_env RESET "$reset"
write_env BOOK_PLUMBER "$book_plumber"
write_env ORDER_FILTERS "$order_filters"
write_env COMPARE_QUOTES "$compare_quotes"
write_env LAUNCH_BRIEF "$launch_brief"
write_env FOLLOW_CHECKS "$follow_checks"
write_env FIRST_PLUMBER_CHECK "$first_plumber_check"
write_env SECOND_PLUMBER_CHECK "$second_plumber_check"

printf 'Wrote %s\n' "$out"
