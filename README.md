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

## Usage
Run `rager help` to see all of the options available to you.

At time of writing, the `view` subcommand doesn't work, and search terms don't yet have Regex support, but these issues should be fixed soon.
