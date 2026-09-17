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
things set-auth
```

Or through the environment, which overrides the auth file:

```sh
export THINGS_EMAIL="you@example.com"
export THINGS_PASSWORD="your-password"
```

The auth file and the sync cache live under `$XDG_STATE_HOME/things`, created
owner-only.

## Usage

```sh
things today
things find --query "rent" --deadline "<=2026-03-31"
things new "Follow up with team" --when today
things schedule <task-id> --deadline 2026-04-10
things mark <task-id> --done
```

Views: `inbox`, `today`, `upcoming`, `anytime`, `someday`, `logbook`,
`projects`, `project <id>`, `areas`, `area <id>`, `tags`, and `find` with title,
notes, checklist, tag, area, project, status and date filters. `--detailed` adds
notes and checklists to any view.

Tasks: `new`, `edit`, `mark`, `schedule`, `reorder` and `delete`, with checklist
operations on `edit` and `mark`. Projects, areas and tags have `new` and `edit`
subcommands.

Not yet supported: reminders and repeat rules, which the sync reads but does not
write.

## Development

`nix develop` provides the toolchain. `nix fmt` runs clippy, rustfmt, deno fmt,
nixpkgs-fmt and taplo, and `nix flake check` runs the same tools read only plus
the test suite. The CLI snapshot suite lives in `tests/cli/` and runs through
`cargo nextest run` or `cargo test`.
