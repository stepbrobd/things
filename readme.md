# Things

A command line client for [Things 3](https://culturedcode.com/things/) using
Things Cloud directly. Hard fork of
[evanpurkhiser/things3-cloud](https://github.com/evanpurkhiser/things3-cloud).

## Run and authenticate

```sh
nix run github:stepbrobd/things -- auth
nix run github:stepbrobd/things -- today
```

Optional binary cache: <https://cache.ysun.co>, key
`cache.ysun.co-1:WxPYwT5g3kt9XhUhHPpNLZKI9HIOsVVAuqSHpok8Qt4=`.

Run `things auth` to save credentials as plaintext in
`$XDG_CONFIG_HOME/things/auth.json`. `THINGS_EMAIL` and `THINGS_PASSWORD`
override the file. The default config directory is `~/.config`.

## Usage

```sh
things today
things show <id>
things find rent --deadline '<=2026-10-31'
things new "Water plants" --when today --repeat daily --reminder 09:00
things edit <id> --when 2026-10-01 --deadline 2026-10-07
things mark <id> --done
things --json upcoming
things completions bash
```

Views: `inbox`, `today` (default), `upcoming`, `anytime`, `someday`, `logbook`,
`projects`, `areas`, `tags`, `project <id>`, and `area <id>`. Use `--detailed`
for notes and checklists, or global `--json` for structured read output.

Mutations: `new`, `edit`, `mark`, `reorder`, and `delete`. Projects, areas, and
tags also have `new` and `edit` subcommands. Use full IDs or unambiguous
prefixes. Run `things <command> --help` for filters, checklist edits, reminders,
and ordering options.

Repeats support `daily`, `weekly:mon,wed,fri`, `monthly:15`, and `yearly`, with
intervals and bounds described in `things new --help`. `edit --repeat` can add a
rule to a non-repeating task but cannot change an existing rule.
After-completion rules such as `after:2w` can be created, but completing them
requires an Apple client.

## Sync and configuration

Loading data syncs first. Even view commands can create due fixed-schedule
repeat instances. There is no background scheduler. If sync fails, commands warn
and use cached data, with writes refused.

The sync cache lives under `$XDG_STATE_HOME/things`, defaulting to
`~/.local/state/things` on every platform. `THINGS_LOG` controls logging, and
`THINGS_LOG_FORMAT` selects `pretty`, `simplified`, or `json`. `NO_COLOR`
disables color.
