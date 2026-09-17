# proc

A process manager that runs the commands defined in a `Procfile`, each in its own PTY, with their output multiplexed into one colored stream — plus an interactive menu for driving them while they run.

```
Procfile
  db: postgres -D ./tmp/db
  web(delay:2s): bin/rails server
  worker(restart, retries:5): bundle exec sidekiq
```

```
$ proc
db     | starting PostgreSQL 16.2
web    | Listening on http://127.0.0.1:3000
worker | Booting Sidekiq 7.2.0
```

Because every process gets a real PTY, programs that check `isatty` behave as they do in your own shell: colors, progress bars and interactive prompts all survive.

## Requirements

- macOS or Linux — `proc` uses Unix PTYs and signals, and does not build on Windows
- Rust 1.98 or newer

## Install

```bash
git clone https://github.com/JWo1F/proc.git
cd proc
cargo install --path .
```

That puts `proc` in `~/.cargo/bin`. To run it without installing, use `cargo run --release --`.

## Quick start

Write a `Procfile` in your project root, one process per line:

```
web: bin/rails server
worker: bundle exec sidekiq
```

Then:

```bash
proc                  # run everything
proc web worker       # run only these
proc check            # validate the file without running anything
proc list             # print what is defined
```

## Procfile format

```
<name>: <command>
<name>(<flags>): <command>
```

Lines beginning with `#` are comments. Each command runs through `$SHELL`, so pipes, `&&` and shell functions all work.

### Process flags

Flags go in parentheses after the name, comma separated, and apply only to the process that declares them:

| Flag | Effect |
| --- | --- |
| `optional` | Kept out of the run until asked for by name, with `-e`/`--enable`, or from the menu |
| `once` | When it exits, leave it down — no restart, and the rest of the run keeps going |
| `restart` | Always respawn it when it exits |
| `stop` | Stop the whole run when it exits |
| `delay:<dur>` | Hold its automatic start (`500ms`, `2s`, `1m`; a bare number means seconds) |
| `retries:<n>` | Give up after n consecutive automatic restarts |
| `muted` | Keep its output out of the terminal |
| `allow-failure` | A non-zero exit from it does not fail the run |

`once`, `restart` and `stop` are the same setting, so only one may appear per process. They override the session-wide `--on-exit` policy for that one process.

```
db: postgres -D ./tmp/db
web(delay:2s): bin/rails server
worker(restart, retries:5): bundle exec sidekiq
logs(muted): tail -f log/development.log
seed(optional, once, allow-failure): bin/rails db:seed
```

Flag names are case-insensitive, and `_` reads the same as `-`, so `allow_failure` works too.

## Interactive mode

In a terminal, `proc` is interactive by default. Press **G** to open a full-screen menu over the log stream; every item has a direct hotkey, and arrows plus Enter work as well.

| Key | Screen |
| --- | --- |
| `P` | Processes — pick one to act on |
| `A` | Add a process to the running session |
| `L` | All processes — bulk start, stop, restart, kill, remove |
| `M` | Session exit mode |
| `D` | Dashboard — aggregate CPU, memory and process counts with graphs |
| `S` | Ps — a live process table |
| `Q` | Quit |

Picking a process opens its own actions: **S**tart, s**T**op, **R**estart, **K**ill, **I**nfo, run m**O**de, **F**ocus, **M**ute, **U**nmute, remove (**X**).

Info shows uptime, restart count, resource use and rolling memory/CPU sparklines. Focus and mute only change what reaches your terminal; they never stop a process.

An explicit stop, kill or remove outranks the `--on-exit` policy, so a process you put down stays down. The session outlives the process set: an empty table is still a prompt you can add to, and processes that have exited stay listed so you can start them again.

Disable all of this with `-I`/`--no-interactive`. Outside a terminal (in CI, or under a pipe) it is off automatically.

## Exit behavior

`--on-exit` decides what happens when a process exits:

- `stop` — stop everything (the default outside interactive mode)
- `restart` — bring it back
- `ignore` — leave it down and quit once all processes are done (the default in interactive mode, since one process finishing should not tear down your session)

A per-process `once`/`restart`/`stop` flag wins over this for that process.

## Pipe mode

Given piped input, `proc` reads stdin and renders it through the same consumer:

```bash
my_command | proc
ssh host "journalctl -f" | proc -T
```

## Options

| Option | Effect |
| --- | --- |
| `-c, --config <PATH>` | Path to the Procfile (default: `Procfile`) |
| `-r, --run <ENTRY>` | Inline `"name: command"`, repeatable; skips the file if no `-c` is given |
| `--on-exit <MODE>` | `restart`, `stop` or `ignore` |
| `-e, --enable <NAME>` | Bring an `optional` process into the run, repeatable |
| `-E, --enable-all` | Bring every `optional` process into the run |
| `-T, --timestamps` | Prefix each line with a timestamp |
| `-s, --compact` | Hide process names, show only colored bars |
| `-q, --silent` | Suppress log output to stdout |
| `--no-system` | Hide system messages (Spawned, Stopped, and so on) |
| `-I, --no-interactive` | Disable interactive mode |

Inline definitions take the same flags as the file:

```bash
proc -r 'extra: sidekiq'
proc -r 'migrate(once): rake db:migrate'
```

## Shutdown

`Ctrl+C` starts a graceful shutdown: every process gets `SIGINT` and five seconds to wind down before `SIGKILL`. A second `Ctrl+C` kills immediately.

`proc` exits non-zero if any process failed, unless that process is marked `allow-failure`.

## Platform notes

Per-process resource figures cover the whole process group — the shell plus everything it spawned. Thread counts are only available on Linux and show as `—` elsewhere.

## License

MIT — see [LICENSE](LICENSE).
