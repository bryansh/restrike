//! Trace format, comparators, and replay driver for differential testing:
//! checking restrike's behavior against traces captured from the original game
//! (D-OR3). This crate owns the `.gbxtrace` format, the projection comparator,
//! the chain-continuity check, and the engine sink that emits traces.
//!
//! This crate is platform-pure and wasm-clean: no windowing, audio, or async
//! runtime dependencies. The canonical writer is byte-deterministic (integers
//! only, fixed field order, no insignificant whitespace) so trace files are
//! hashable; the reader is liberal (ignores unknown/extra fields) so the
//! step-3 staging hook's diagnostic fields don't break it.
//!
//! Layout:
//! - [`format`] — the `.gbxtrace` types, canonical writer, liberal parser.
//! - [`compare`] — the validity gate + projection equality, and the
//!   single-trace chain-continuity check.
//! - [`sink`] — the [`gbx_engine::rng::RngSink`] implementation that turns live
//!   engine draws into a trace.
//!
//! Action-profile vocabulary lands incrementally as combat systems do (D-OR3,
//! step 5). The **initiative slice** pins [`format::InitEvent`] /
//! [`format::PickEvent`] (`init`/`pick`) and the [`sink::TraceCollector`] action
//! sink that emits them; the remaining action types (`attack`/`dmg`/`move`/`ai`/
//! `status`/`award`) are still open and their sessions add them. Equality over
//! action events is not yet a gate (§9) — the profile, versioning, and same-tick
//! emission-order contract are what exist.

pub mod compare;
pub mod format;
pub mod sink;

pub use compare::{check_chain, compare, ChainBreak, Comparison, Divergence, Incomparable};
pub use format::{
    InitEvent, ParseError, PickEvent, Profile, RngEvent, Trace, TraceEvent, TraceHeader,
};
pub use sink::TraceCollector;
