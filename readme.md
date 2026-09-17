# things

A command-line client for [Things 3](https://culturedcode.com/things/) that
talks to the Things Cloud API directly, so it runs anywhere, Linux included. A
hard fork of
[evanpurkhiser/things3-cloud](https://github.com/evanpurkhiser/things3-cloud):
the protocol work started there, this repository carries the nix packaging, the
`things` binary and the additions below, and nothing goes back upstream.

Binary Cache:

- Cache: <https://cache.ysun.co>
- Key: `cache.ysun.co-1:WxPYwT5g3kt9XhUhHPpNLZKI9HIOsVVAuqSHpok8Qt4=`

Run without installing:

```sh
nix run github:stepbrobd/things -- today
```

## Auth

```sh
things auth
```

Or through the environment, which overrides the auth file:

```sh
export THINGS_EMAIL="you@example.com"
export THINGS_PASSWORD="your-password"
```

The auth file lives at `$XDG_CONFIG_HOME/things/auth.json` and the sync cache
under `$XDG_STATE_HOME/things`, both created owner-only. The variables are
honored on every platform, macOS included, with `~/.config` and `~/.local/state`
as the defaults.

## Usage

```sh
things today
things show <task-id>
things find --query "rent" --deadline "<=2026-03-31"
things new "Follow up with team" --when today
things new "Water plants" --when today --repeat daily --reminder 09:00
things new "Pay rent" --when 2026-10-01 --repeat monthly:1 --times 12
things edit <task-id> --when 2026-04-10 --reminder 09:00 --deadline 2026-04-17
things mark <task-id> --done
```

Views: `inbox`, `today`, `upcoming`, `anytime`, `someday`, `logbook`,
`projects`, `project <id>`, `areas`, `area <id>`, `tags`, `show <id>`, and
`find` with title, notes, checklist, tag, area, project, status and date
filters. `--detailed` adds notes and checklists to any list, `--json` prints any
view as JSON.

Changes: `new` creates a to-do, `edit` changes its title, notes, container,
tags, checklist, when, deadline, reminder or repeat, `mark` sets it done,
incomplete or canceled, `reorder` moves it relative to another, `delete` trashes
it, a project or area with its contents. Projects, areas and tags have `new` and
`edit` subcommands.

Reminders are set with `--reminder HH:MM` on `new` and `edit`, cleared with
`--clear-reminder`, and shown as `@HH:MM` on task lines.

Repeat rules are set with `--repeat` on `new` and `edit`: `daily`, `weekly`,
`weekly:mon,thu`, `monthly:15`, `monthly:last`, `yearly:12-31` or `after:2w` for
after completion, with `/N` for every N units as in `weekly/2:sat`. `--times N`
or `--until YYYY-MM-DD` bound the rule. The to-do needs a when date, which moves
to the first matching day, and becomes the first instance of a hidden template
exactly as the app writes it, so the following instances appear on their days:
every sync creates the instances whose day has come, the way the Apple clients
do. `upcoming` also lists each repeating to-do on its next day, marked `↻`,
before that instance exists.

## Environment

- `THINGS_EMAIL`, `THINGS_PASSWORD`: Things Cloud credentials, over the auth
  file.
- `THINGS_LOG`: a log filter directive such as `debug`, off by default beyond
  errors. `THINGS_LOG_FORMAT` picks `pretty`, `simplified` or `json`.
- `NO_COLOR`: disables color, which otherwise follows whether stdout is a
  terminal.
- `XDG_CONFIG_HOME`, `XDG_STATE_HOME`: where the auth file and the sync log
  live.

When the sync fails, offline or otherwise, the command warns once and runs on
the cached state. Every commit the server returns is appended to `things.log`
under the state directory, one line per commit, which is the record to read when
something looks wrong.

## Development

`nix develop` provides the toolchain. `nix fmt` runs clippy, rustfmt, deno fmt,
nixpkgs-fmt and taplo, and `nix flake check` runs the test suite. The CLI
snapshot suite lives in `tests/cli/` and runs through `cargo nextest run` or
`cargo test`; its runner pins the clock, the ids and the log filter so payloads
can be asserted.
