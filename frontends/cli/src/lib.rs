//! `restrike-cli`'s library half: the parts of the CLI worth calling
//! in-process (from integration tests, or eventually other tooling)
//! instead of only through the `restrike` binary. Everything else the CLI
//! does stays a private module of `src/main.rs`.

pub mod walk;
