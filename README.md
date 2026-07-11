# Restrike

The SSI Gold Box RPGs, struck again from the original data.

Restrike is a native, cross-platform reimplementation of SSI's "Gold Box"
engine — the shared engine behind *Pool of Radiance*, *Curse of the Azure
Bonds*, the *Buck Rogers* games, and the rest of the family. It follows the
same model as ScummVM and OpenMW: instead of emulating the original 1990 DOS
machine, it re-implements the engine itself and loads the original game's
data files directly, so the games run natively on modern hardware — desktop
today, WebAssembly in the browser down the line.

**This repository is engine only.** It contains no game data, art, text, or
scripts from any SSI title — you supply your own legally-obtained copy of a
Gold Box game and point the engine at it.

## Build & run

Requires a stable Rust toolchain (see `rust-toolchain.toml`; `rustup` will
pick it up automatically).

```sh
cargo build --workspace
cargo test --workspace
```

Point the CLI at a game data directory to identify it:

```sh
cargo run -p restrike-cli -- detect /path/to/your/game/data
# or, via the environment:
GBX_DATA_DIR=/path/to/your/game/data cargo run -p restrike-cli -- detect
```

## Status

Early scaffold (milestone M0). See [PLAN.md](PLAN.md) for the full
implementation plan, architecture, and milestone roadmap, and
[SOURCES.md](SOURCES.md) for the provenance ledger of every external
reference used along the way.

## License

[GPL-3.0-or-later](LICENSE).
