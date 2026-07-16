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
//! What this session leaves open (a combat session closes it): the `action`
//! profile's event *vocabulary* (`init`/`attack`/`dmg`/`move`/`ai`/`status`/
//! `award`). The profile, the versioning, and the same-tick emission-order
//! contract exist ([`format::Profile::Action`]); the event fields are
//! deliberately not invented ahead of the combat systems that pin them.

pub mod compare;
pub mod format;
pub mod sink;

pub use compare::{check_chain, compare, ChainBreak, Comparison, Divergence, Incomparable};
pub use format::{ParseError, Profile, RngEvent, Trace, TraceEvent, TraceHeader};
pub use sink::TraceCollector;
