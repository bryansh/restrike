# gbx-formats fuzz targets

Untrusted-input parsers get a `cargo-fuzz` target each (PLAN.md M1's fuzz
roster). This directory is its own detached Cargo workspace (`[workspace]`
in `fuzz/Cargo.toml`) per the standard `cargo-fuzz` layout — it is not a
member of the repo's top-level workspace and CI does not build or run it.

## Running

```sh
cargo install cargo-fuzz   # once
cargo +nightly fuzz run exepack
```

(`cargo-fuzz` requires a nightly toolchain for the sanitizer instrumentation
— unrelated to the stable toolchain the rest of this repo builds with.)

Stop with Ctrl-C; a crashing input, if found, is saved under
`fuzz/artifacts/exepack/` for replay via `cargo +nightly fuzz run exepack
<path-to-artifact>`.
