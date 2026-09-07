#!/usr/bin/env bash
set -euo pipefail

work_dir=${DEMO_WORK_DIR:-/tmp/things3-demo-gif}
cast=${1:-$work_dir/things3-cli-demo.cast}
gif=${2:-$work_dir/things3-cli-demo.gif}

"${DEMO_AGG_BIN:-agg}" \
	--font-dir "${DEMO_FONT_DIR:-$HOME/.local/share/fonts}" \
	--text-font-family "${DEMO_TEXT_FONT_FAMILY:-Cascadia Mono,Noto Sans Symbols 2}" \
	--emoji-font-family "${DEMO_EMOJI_FONT_FAMILY:-Noto Emoji,Noto Color Emoji,Apple Color Emoji,Segoe UI Emoji}" \
	--font-size "${DEMO_FONT_SIZE:-22}" \
	--line-height "${DEMO_LINE_HEIGHT:-1.4}" \
	--theme "${DEMO_THEME:-github-light}" \
	--speed "${DEMO_SPEED:-1}" \
	--idle-time-limit "${DEMO_IDLE_TIME_LIMIT:-3}" \
	"$cast" \
	"$gif"

printf 'Wrote %s\n' "$gif"
