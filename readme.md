# Things

`things` is an unofficial command-line client for
[Things 3](https://culturedcode.com/things/) that reads and writes Things Cloud
directly. It works without Things.app and is a hard fork of
[evanpurkhiser/things3-cloud](https://github.com/evanpurkhiser/things3-cloud).

Cultured Code does not publish or support the Things Cloud API. Protocol changes
may break this client.

## Run

```nu
nix run github:stepbrobd/things -- auth
nix run github:stepbrobd/things -- today
```

Prebuilt Nix outputs are available from this cache:

- Cache: <https://cache.ysun.co>
- Public key: `cache.ysun.co-1:WxPYwT5g3kt9XhUhHPpNLZKI9HIOsVVAuqSHpok8Qt4=`

`things auth` signs in to Things Cloud with the email and password it asks for
and saves them only when the sign-in succeeds, as plaintext JSON in
`$XDG_CONFIG_HOME/things/auth.json`. The default path is
`~/.config/things/auth.json`, and the file is created with mode `0600` on Unix.
`THINGS_EMAIL` and `THINGS_PASSWORD` override the corresponding fields.

## Use

```nu
things
things show ABCD
things find rent --deadline "<=2026-10-31"
things new "Water plants" --when today --repeat daily --reminder 09:00
things edit ABCD --when 2026-10-01 --deadline 2026-10-07
things mark ABCD --done
things --json upcoming
things completions nushell
```

Running `things` without a subcommand shows Today. `ABCD` above represents a
full ID or a unique prefix.

The other views are `inbox`, `upcoming`, `anytime`, `someday`, `logbook`,
`projects`, `areas`, `tags`, `project ID`, and `area ID`. Views that accept
`--detailed` use it to include notes and checklists. Add the global `--json`
option for structured output where the selected command supports it.

Task mutations use `new`, `edit`, `mark`, `reorder`, and `delete`. The
`projects` and `areas` commands provide `list`, `new`, and `edit` subcommands.
`tags` also provides `delete`.

Run `things <command> --help` for filters, checklist operations, scheduling,
reminders, and ordering.

## Repeat rules

`--repeat` accepts these forms:

- `daily`
- `weekly` or `weekly:mon,wed,fri`
- `monthly:15` or `monthly:last`
- `yearly` or `yearly:12-31`
- `after:2w`, with `d`, `w`, `m`, and `y` units

Put `/N` after a fixed cadence to repeat every N periods, as in `daily/3` or
`weekly/2:sat`. `--times N` and `--until YYYY-MM-DD` provide mutually exclusive
bounds.

A new task with a repeat requires `--when`. Adding a repeat with `edit` requires
an existing or newly assigned scheduled day. The first occurrence must be today
or later. Without a selector, that day supplies the weekday, day of the month,
or month and day. Checklists are copied to the repeat template and each
generated instance.

Repeat rules have these limits:

- A task with a deadline cannot receive a new repeat rule. Things stores the
  repeating deadline as an offset whose wire format has not been verified
  against an Apple client.
- An `after:` rule can be created and displayed. Completing it requires an Apple
  client.
- A rule created by an Apple client remains visible when the CLI cannot evaluate
  its shape exactly. Its instances are left to Apple clients.
- A template is also left to Apple clients when its recorded next day differs
  from the day computed from its rule.
- A template inside a completed or trashed project or heading creates no
  instances.

## Sync and local state

Every command that reads account data syncs before it runs. The CLI then creates
any due fixed-schedule repeat instances before running the selected command,
including a view command. There is no background scheduler.

The sync cache is in `$XDG_STATE_HOME/things`, which defaults to
`~/.local/state/things`. It is bound to the authenticated account. Changing
accounts fetches that account's history from the beginning. Concurrent runs
serialize access to the cache, and a run holds it until its repeat pass has
committed.

When sync fails, the command warns, reads cached state and exits with status 3
instead of 0. Writes are refused for that run. If an object's history does not
replay completely, the CLI reports it, refuses writes to that object, and sets
`flags.degraded` in JSON output.

Histories from before Things Cloud moved to base58 ids are not supported.
Objects keyed by UUIDs are skipped, and an object that the `Task3`, `Task4`,
`Area2` or first `Tombstone` kinds reach, or that holds a note stored as a plain
XML string, is reported as not replayed.

The selected command exits with status 1 on failure and 2 on invalid arguments.
Batch mutations validate every target before their own write. The repeat pass
runs first and may create due instances even when the selected command later
fails.

`THINGS_LOG` sets the log filter, which shows errors alone by default.
`THINGS_LOG=warn` lists the objects behind a replay notice. `THINGS_LOG_FORMAT`
selects `pretty`, `simplified`, or `json`. `NO_COLOR` disables color.

## License

[MIT](license.txt)
