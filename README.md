# Rager

A CLI Tool for syncing/viewing/searching Matrix [Rageshake server](https://github.com/matrix-org/rageshake) submissions.

To make it work correctly, you need to place a config file (similar to the [rager.toml](./rager.toml) config file here) at the config directory of your user directory. If you don't know where that would be or are unsure, Just run `rager sync` and it will tell you where the config file should be.

## Building
As with all other rust project, [install the rust toolchain](https://rustup.rs), then run:

```
git clone https://github.com/iandwelker/rager.git
cd rager
cargo build --release
```

You can also grab a pre-compiled binary from the **Releases** section, but those may not be fully up to date with the changes in this repo.

## Usage
Run `rager help` to see all of the options available to you.

```
$ rager help

Rager 1.0
Ian Welker <@ian:beeper.com>

USAGE:
    rager [SUBCOMMAND]

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

SUBCOMMANDS:
    complete    List completions for view command
    desync      Clear all logs off of your device
    help        Prints this message or the help of the given subcommand(s)
    prune       Delete all entries that match the terms
    search      Search through the logs currently on your device
    sync        Download all the logs from the server that you don't currently have on your device
    view        View a specific Entry

```

## Completion

Rager offers shell tab completions for the `view` subcommand, as long as you run `rager complete --install` and follow its prompts.
