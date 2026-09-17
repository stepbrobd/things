# things

A command-line client for [Things 3](https://culturedcode.com/things/) that
talks to the Things Cloud API directly, so it runs anywhere, Linux included.
Forked from
[evanpurkhiser/things3-cloud](https://github.com/evanpurkhiser/things3-cloud),
which stays the upstream: protocol work flows from there, this fork carries the
nix packaging, the `things` binary name and the additions listed below.

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
things find --query "rent" --deadline "<=2026-03-31"
things new "Follow up with team" --when today
things schedule <task-id> --deadline 2026-04-10
things schedule <task-id> --when 2026-04-10 --reminder 09:00
things new "Water plants" --when today --repeat daily --reminder 09:00
things new "Pay rent" --when 2026-10-01 --repeat monthly:1 --times 12
things mark <task-id> --done
```

Views: `inbox`, `today`, `upcoming`, `anytime`, `someday`, `logbook`,
`projects`, `project <id>`, `areas`, `area <id>`, `tags`, and `find` with title,
notes, checklist, tag, area, project, status and date filters. `--detailed` adds
notes and checklists to any view.

Tasks: `new`, `edit`, `mark`, `schedule`, `reorder` and `delete`, with checklist
operations on `edit` and `mark`. Projects, areas and tags have `new` and `edit`
subcommands.

Reminders are set with `--reminder HH:MM` on `new` and `schedule`, cleared with
`--clear-reminder`, and shown as `@HH:MM` on task lines.

Repeat rules are set with `--repeat` on `new` and `schedule`: `daily`, `weekly`,
`weekly:mon,thu`, `monthly:15`, `monthly:last`, `yearly:12-31` or `after:2w` for
after completion, with `/N` for every N units as in `weekly/2:sat`. `--times N`
or `--until YYYY-MM-DD` bound the rule. The to-do needs a when date, which moves
to the first matching day, and becomes the first instance of a hidden template
exactly as the app writes it, so the following instances appear on their days:
every sync creates the instances whose day has come, the way the Apple clients
do. `upcoming` also lists each repeating to-do on its next day, marked `↻`,
before that instance exists.

## Development

`nix develop` provides the toolchain. `nix fmt` runs clippy, rustfmt, deno fmt,
nixpkgs-fmt and taplo, and `nix flake check` runs the same tools read only plus
the test suite. The CLI snapshot suite lives in `tests/cli/` and runs through
`cargo nextest run` or `cargo test`.
