# Contributing

Thanks for helping! Questions and ideas are welcome on
[Discord](https://discord.gg/sTzwBzDkK9) or in an issue.

## Setup

- Rust (stable), GTK 4.14+ development files (`libgtk-4-dev`, `gtk4-devel`, `gtk4`)
- [Deno](https://deno.com) 2

```sh
cargo run                                    # the launcher, using ./sdk and your installed extensions
BACKSLASH_EXTENSIONS_PATH=$PWD/extensions cargo run   # with the bundled extensions
cargo run -- run npm/search react            # CLI
make check                                   # everything CI runs
```

`backslash dev <dir>` runs a separate instance (`app.backslash.Backslash.Devel`)
that never hides on focus loss and prints extension logs.

## Where things are

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first. In short:

- `crates/backslash-core`: protocol, host, ranking, manifests. Most logic, most tests.
- `crates/backslash`: the GTK window and the CLI.
- `sdk/`: the TypeScript API and its runtime.
- `extensions/`: bundled extensions.

Protocol changes must update `protocol.rs`, `sdk/`, and `docs/PROTOCOL.md` together.

## Pull requests

- Keep them focused; describe the user-visible change.
- `make check` must pass (format, clippy, tests, Deno lint and type check).
- New behavior in `backslash-core` comes with a test.
