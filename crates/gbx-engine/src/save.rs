//! The `.rsav` envelope (`docs/design/save-formats.md` D-SAVE1..4, task
//! deliverable 3): a hand-encoded little-endian [`ContainerHeader`] wrapping
//! `postcard`-serialized [`SaveState`]. `Engine::save`/`Engine::restore`
//! (in `engine.rs`) are the only public entry points frontends need — this
//! module owns the format itself.
//!
//! **Determinism (D-SAVE1, CI-enforced):** every collection reachable from
//! [`SaveState`] is a `BTreeMap`/`BTreeSet`/`Vec`, never a `HashMap`/
//! `HashSet` — [`crate::vmhost::WindowsSnapshot`] is the prime case
//! (`gbx_vm::VmString` inside a `BTreeMap`, not the live `HashMap` the
//! runtime `VmMemoryState` uses internally for performance). No floats: the
//! one candidate (the text pacer's fractional accumulator) is already
//! stored as fixed-point millis (`crate::text::TextPacer`), not `f32`.
//!
//! **Two documented, real scope gaps** (not silently omitted — flagged
//! per this project's own discipline): D-SAVE3 names "the active
//! animation's frame index + countdown" and "game_speed" as pixel-affecting
//! state a save must carry. Neither exists as *live, mutable* engine state
//! yet — `Effect::AnimationFrame` has no consumer beyond a no-op match arm
//! (`shell.rs`), and the text pacer's speed is a boot-time constant
//! (`TextPacer::new(4)`, `engine.rs`), not a runtime-settable `game_speed`.
//! There is nothing to serialize for either until a future session adds the
//! feature; `SaveState` will grow a field then, bumping
//! [`SAVE_FORMAT_VERSION`] (D-SAVE2).

use crate::engine::Engine;
use crate::party::Party;
use crate::rng::EngineRng;
use crate::shell::{EngineState, Shell};
use crate::text::{TextCursor, TextPacer};
use crate::vmhost::WindowsSnapshot;
use gbx_formats::game_data::GameData;
use gbx_vm::COTAB;
use sha2::{Digest, Sha256};
use std::fmt;

/// `b"RSAV"` (D-SAVE1/§3).
pub const CONTAINER_MAGIC: [u8; 4] = *b"RSAV";
/// Header layout version — checked first, before the payload is even
/// looked at (D-SAVE2).
pub const CONTAINER_VERSION: u16 = 1;
/// The payload's single version authority (D-SAVE2): subsumes every nested
/// snapshot's own tag (`gbx_vm::Snapshot`'s `SNAPSHOT_VERSION` among them).
/// Bump on any serialization-incompatible change to `SaveState` or anything
/// it contains.
pub const SAVE_FORMAT_VERSION: u32 = 1;

/// This engine's one shipped flavor (M3's slice — `xxvc` is M7). An 8-byte
/// ASCII tag rather than a numeric id, matching the header's own
/// "greppable without deserializing the payload" design goal (D-SAVE1/§3).
pub const FLAVOR_ADND1: [u8; 8] = *b"ADND1\0\0\0";

/// The fixed-size, hand-encoded little-endian header (D-SAVE1/§3's exact
/// field list) — deliberately not `repr(C)`/`transmute` (neither pins
/// endianness nor forbids padding). `container_version` is checked before
/// anything else is even parsed (D-SAVE2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerHeader {
    pub container_version: u16,
    pub save_format_version: u32,
    /// Selector: rejected outright if it doesn't match this build's one
    /// flavor (D-SAVE2) — there is no dynamic re-binding to reject-not-
    /// migrate *to* yet, since this codebase ships exactly one flavor.
    pub flavor: [u8; 8],
    /// The detection-table hash of the `GameData` this save was made
    /// against — load-bearing (D-SAVE2): a save against stale/different
    /// data is rejected, not silently corrupted.
    pub data_fingerprint: [u8; 32],
    /// Provenance only (D-SAVE4) — resume uses `SaveState.prng`, never this.
    pub seed: u64,
    /// Provenance only (D-SAVE4) — a session/tick coordinate, not a trace
    /// binding.
    pub tick_index: u64,
    pub payload_len: u64,
}

/// `4 (magic) + 2 + 4 + 8 + 32 + 8 + 8 + 8`.
const HEADER_LEN: usize = 4 + 2 + 4 + 8 + 32 + 8 + 8 + 8;

impl ContainerHeader {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN);
        out.extend_from_slice(&CONTAINER_MAGIC);
        out.extend_from_slice(&self.container_version.to_le_bytes());
        out.extend_from_slice(&self.save_format_version.to_le_bytes());
        out.extend_from_slice(&self.flavor);
        out.extend_from_slice(&self.data_fingerprint);
        out.extend_from_slice(&self.seed.to_le_bytes());
        out.extend_from_slice(&self.tick_index.to_le_bytes());
        out.extend_from_slice(&self.payload_len.to_le_bytes());
        debug_assert_eq!(out.len(), HEADER_LEN);
        out
    }

    /// Parses the header and returns it alongside the remaining bytes (the
    /// payload). `container_version` is validated here, first — matching
    /// D-SAVE2's "the header must parse before the payload" ordering; the
    /// payload's own `save_format_version` is checked by the caller
    /// ([`load`]) once the header itself is known-good.
    pub fn decode(bytes: &[u8]) -> Result<(Self, &[u8]), SaveError> {
        if bytes.len() < HEADER_LEN {
            return Err(SaveError::Truncated {
                need: HEADER_LEN,
                got: bytes.len(),
            });
        }
        if bytes[0..4] != CONTAINER_MAGIC {
            return Err(SaveError::BadMagic);
        }
        let container_version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        if container_version != CONTAINER_VERSION {
            return Err(SaveError::UnknownContainerVersion {
                found: container_version,
                expected: CONTAINER_VERSION,
            });
        }
        let save_format_version = u32::from_le_bytes(bytes[6..10].try_into().unwrap());
        let mut flavor = [0u8; 8];
        flavor.copy_from_slice(&bytes[10..18]);
        let mut data_fingerprint = [0u8; 32];
        data_fingerprint.copy_from_slice(&bytes[18..50]);
        let seed = u64::from_le_bytes(bytes[50..58].try_into().unwrap());
        let tick_index = u64::from_le_bytes(bytes[58..66].try_into().unwrap());
        let payload_len = u64::from_le_bytes(bytes[66..74].try_into().unwrap());
        debug_assert_eq!(74, HEADER_LEN);
        let header = ContainerHeader {
            container_version,
            save_format_version,
            flavor,
            data_fingerprint,
            seed,
            tick_index,
            payload_len,
        };
        Ok((header, &bytes[HEADER_LEN..]))
    }
}

/// The full restorable engine state (D-SAVE3) — the `.rsav` payload,
/// `postcard`-serialized. serde-derived throughout (that derive *is* the
/// storage mechanism, D-SAVE1); every nested type's own collections are
/// deterministic (module doc comment).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SaveState {
    /// D-VM3 verbatim: resident block bytes, activation stack + pendings,
    /// compare flags, string registers, call stack.
    pub ecl: gbx_vm::Snapshot,
    /// D-UI2: `Shell`/`VmPhase`/`Widget`, flow-plan cursors, `chained`,
    /// the presentation queue and every parked `TextJob`'s emit-progress
    /// (both already embedded inside `VectorRun`, itself inside the active
    /// flow stage — no separate wrapper needed).
    pub shell: Shell,
    /// `game_state`/position/`search_flags`/`chained`/`party_killed`/the
    /// game clock/etc — the D-UI1/D-UI2 M3 engine-state slice.
    pub state: EngineState,
    /// `SetEgaPalette` remaps (D-SAVE3) — pixels themselves are not stored
    /// (re-composited by a render-all on restore).
    pub palette: [[u8; 3]; 16],
    pub cursor: TextCursor,
    pub pacer: TextPacer,
    /// Area/Party/Table/Global window backings (named cells + raw
    /// fallback, D-VM5) plus resident-asset ids (`setBlocks`, 3D map,
    /// bigpic) — deterministic collections only (D-SAVE1). Excludes the
    /// unknown-access log and other diagnostics (D-SAVE3).
    pub windows: WindowsSnapshot,
    /// The party roster (D-SAVE11, task deliverable 2).
    pub party: Party,
    /// D9: the engine's one PRNG's internal state — resume continues the
    /// exact roll sequence (D-SAVE4).
    pub prng: EngineRng,
}

/// [`ContainerHeader::decode`]/[`load`]/[`Engine::restore`]'s failure mode.
/// Every variant carries what a user-facing diagnostic needs (D-SAVE2:
/// "which version the save is, which the binary expects"). Reject-not-
/// migrate, always — never a silent best-effort load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    Truncated {
        need: usize,
        got: usize,
    },
    BadMagic,
    UnknownContainerVersion {
        found: u16,
        expected: u16,
    },
    UnknownSaveFormatVersion {
        found: u32,
        expected: u32,
    },
    UnknownFlavor {
        found: [u8; 8],
    },
    /// The save was made against different `GameData` than is currently
    /// loaded (D-SAVE2 — load-bearing, not advisory).
    DataFingerprintMismatch,
    PayloadLengthMismatch {
        header_says: u64,
        actual: u64,
    },
    Deserialize(String),
    Boot(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Truncated { need, got } => {
                write!(
                    f,
                    "save file too short: need at least {need} bytes, got {got}"
                )
            }
            SaveError::BadMagic => write!(f, "not a restrike save file (bad magic)"),
            SaveError::UnknownContainerVersion { found, expected } => write!(
                f,
                "save container version {found} is not supported by this build (expects {expected})"
            ),
            SaveError::UnknownSaveFormatVersion { found, expected } => write!(
                f,
                "save format version {found} is not supported by this build (expects {expected}) \
                 — reject-not-migrate: load with the matching binary version instead"
            ),
            SaveError::UnknownFlavor { found } => {
                let tag = String::from_utf8_lossy(found);
                write!(f, "save names an unavailable flavor {tag:?}")
            }
            SaveError::DataFingerprintMismatch => write!(
                f,
                "this save was made against a different game data set — re-detect GBX_DATA_DIR \
                 or use the original data"
            ),
            SaveError::PayloadLengthMismatch {
                header_says,
                actual,
            } => write!(
                f,
                "save payload length mismatch: header says {header_says}, file has {actual}"
            ),
            SaveError::Deserialize(msg) => write!(f, "corrupt save payload: {msg}"),
            SaveError::Boot(msg) => write!(f, "could not reload assets for this save: {msg}"),
        }
    }
}

impl std::error::Error for SaveError {}

/// A fingerprint of `data`'s exact file set (PLAN §2.3) — SHA-256 over
/// every loaded file's uppercased name and bytes, in `GameData`'s own
/// sorted order (`BTreeMap`-backed, so this is already deterministic).
/// Distinct from `gbx_formats::detect`'s per-file/named-game report: this
/// is a single combined digest for the "is this the exact data set a save
/// was made against" check (D-SAVE2), not game identification.
pub fn data_fingerprint(data: &GameData) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for name in data.file_names() {
        hasher.update(name.as_bytes());
        hasher.update([0u8]); // separator, so no two (name,bytes) pairs can collide by concatenation
        if let Some(bytes) = data.raw_file(name) {
            hasher.update(bytes);
        }
    }
    hasher.finalize().into()
}

/// Encodes `state` behind a header naming `data`'s fingerprint (D-SAVE2)
/// and the given provenance (D-SAVE4) — the `.rsav` write path.
pub fn encode(state: &SaveState, data: &GameData, seed: u64, tick_index: u64) -> Vec<u8> {
    let payload =
        postcard::to_allocvec(state).expect("SaveState has no fallible/unbounded serialization");
    let header = ContainerHeader {
        container_version: CONTAINER_VERSION,
        save_format_version: SAVE_FORMAT_VERSION,
        flavor: FLAVOR_ADND1,
        data_fingerprint: data_fingerprint(data),
        seed,
        tick_index,
        payload_len: payload.len() as u64,
    };
    let mut out = header.encode();
    out.extend_from_slice(&payload);
    out
}

/// Decodes and validates a `.rsav` file's header + payload against `data`
/// (D-SAVE2's full reject-not-migrate chain: container version, then
/// save-format version, then flavor, then the data fingerprint) — returns
/// the decoded [`SaveState`] plus the header (callers that only need
/// provenance/diagnostics don't have to re-parse it).
pub fn load(bytes: &[u8], data: &GameData) -> Result<(ContainerHeader, SaveState), SaveError> {
    let (header, payload) = ContainerHeader::decode(bytes)?;
    if header.save_format_version != SAVE_FORMAT_VERSION {
        return Err(SaveError::UnknownSaveFormatVersion {
            found: header.save_format_version,
            expected: SAVE_FORMAT_VERSION,
        });
    }
    if header.flavor != FLAVOR_ADND1 {
        return Err(SaveError::UnknownFlavor {
            found: header.flavor,
        });
    }
    if header.data_fingerprint != data_fingerprint(data) {
        return Err(SaveError::DataFingerprintMismatch);
    }
    if payload.len() as u64 != header.payload_len {
        return Err(SaveError::PayloadLengthMismatch {
            header_says: header.payload_len,
            actual: payload.len() as u64,
        });
    }
    let state: SaveState =
        postcard::from_bytes(payload).map_err(|e| SaveError::Deserialize(e.to_string()))?;
    Ok((header, state))
}

/// Rebuilds a live [`Engine`] from a decoded [`SaveState`] plus `data` —
/// shared by [`Engine::restore`] and (deliverable 4) `import_original`'s
/// assembly step, since both are "given engine state + `GameData`,
/// reconstruct a running `Engine`". Resident assets (GEO block, wallsets)
/// are re-fetched from `data` by the ids `windows.assets` carries (D-SAVE3:
/// "resident-asset ids, not bytes") — never trusted from stored pixels,
/// because there are none stored. The VM dialect is always
/// [`gbx_vm::COTAB`] (this codebase's one shipped dialect; the header's
/// flavor tag is validated by [`load`] before this is ever called).
pub(crate) fn rebuild_engine(
    header: &ContainerHeader,
    state: SaveState,
    data: GameData,
) -> Result<Engine, SaveError> {
    let assets = crate::boot::boot(&data).map_err(|e| SaveError::Boot(format!("{e:?}")))?;
    let mut symbol_sets = assets.symbol_sets;

    let geo_block_id = state
        .windows
        .assets
        .map_3d_block
        .unwrap_or(crate::engine::DEFAULT_GEO_BLOCK);
    let geo = crate::vmhost::load_geo_block(&data, crate::engine::GAME_AREA, geo_block_id)
        .map_err(|e| SaveError::Boot(format!("{e:?}")))?;

    crate::vmhost::reload_walldefs(
        &mut symbol_sets,
        &data,
        crate::engine::GAME_AREA,
        &state.windows.assets.walldefs,
    );

    let machine = gbx_vm::EclMachine::restore(state.ecl, &COTAB)
        .map_err(|e| SaveError::Boot(format!("{e:?}")))?;

    let mut vm_memory = crate::vmhost::VmMemoryState::new();
    vm_memory.restore_windows(state.windows);

    let mut fb = crate::framebuffer::Framebuffer::new();
    fb.set_palette(state.palette);

    // D-RP4: recomputed fresh on every boot/restore, never serialized.
    let verify_report = gbx_rules::pack::RuleSet::load().verify(&data);

    Ok(Engine::assemble(crate::engine::AssembledEngine {
        fb,
        font: assets.font,
        geo,
        data,
        shell: state.shell,
        state: state.state,
        machine,
        vm_memory,
        party: state.party,
        rng: state.prng,
        cursor: state.cursor,
        pacer: state.pacer,
        symbol_sets,
        sky: assets.sky,
        verify_report,
        boot_seed: header.seed,
        tick_count: header.tick_index,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_data() -> GameData {
        GameData::from_files([("FOO.DAT".to_string(), vec![1, 2, 3])])
    }

    fn synthetic_state() -> SaveState {
        let mut b = gbx_vm::test_support::EclBuilder::new();
        b.op(0x00); // EXIT
        let block = b.build();
        let machine =
            gbx_vm::EclMachine::load_block(block, &COTAB).unwrap_or_else(|never| match never {});
        SaveState {
            ecl: machine.snapshot(),
            shell: crate::shell::Shell::GameOver,
            state: EngineState::new(),
            palette: gbx_rules::palette::EGA_PALETTE,
            cursor: TextCursor::new(),
            pacer: TextPacer::new(4),
            windows: WindowsSnapshot::default(),
            party: Party::default(),
            prng: EngineRng::new(42),
        }
    }

    #[test]
    fn header_round_trips_byte_for_byte() {
        let header = ContainerHeader {
            container_version: CONTAINER_VERSION,
            save_format_version: SAVE_FORMAT_VERSION,
            flavor: FLAVOR_ADND1,
            data_fingerprint: [7u8; 32],
            seed: 123,
            tick_index: 456,
            payload_len: 789,
        };
        let bytes = header.encode();
        assert_eq!(bytes.len(), HEADER_LEN);
        let (decoded, rest) = ContainerHeader::decode(&bytes).unwrap();
        assert_eq!(decoded, header);
        assert!(rest.is_empty());
    }

    #[test]
    fn decode_rejects_truncated_bytes() {
        let err = ContainerHeader::decode(&[0u8; 10]).unwrap_err();
        assert_eq!(
            err,
            SaveError::Truncated {
                need: HEADER_LEN,
                got: 10
            }
        );
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut bytes = vec![0u8; HEADER_LEN];
        bytes[0..4].copy_from_slice(b"NOPE");
        let err = ContainerHeader::decode(&bytes).unwrap_err();
        assert_eq!(err, SaveError::BadMagic);
    }

    #[test]
    fn decode_rejects_unknown_container_version() {
        let mut header = ContainerHeader {
            container_version: CONTAINER_VERSION,
            save_format_version: SAVE_FORMAT_VERSION,
            flavor: FLAVOR_ADND1,
            data_fingerprint: [0u8; 32],
            seed: 0,
            tick_index: 0,
            payload_len: 0,
        };
        header.container_version = 0xFFFF;
        let bytes = header.encode();
        let err = ContainerHeader::decode(&bytes).unwrap_err();
        assert_eq!(
            err,
            SaveError::UnknownContainerVersion {
                found: 0xFFFF,
                expected: CONTAINER_VERSION
            }
        );
    }

    #[test]
    fn load_rejects_unknown_save_format_version() {
        let data = synthetic_data();
        let state = synthetic_state();
        let mut bytes = encode(&state, &data, 1, 1);
        // Corrupt save_format_version in place (bytes 6..10 of the header).
        bytes[6..10].copy_from_slice(&999u32.to_le_bytes());
        let err = load(&bytes, &data).unwrap_err();
        assert_eq!(
            err,
            SaveError::UnknownSaveFormatVersion {
                found: 999,
                expected: SAVE_FORMAT_VERSION
            }
        );
    }

    #[test]
    fn load_rejects_data_fingerprint_mismatch() {
        let data = synthetic_data();
        let state = synthetic_state();
        let bytes = encode(&state, &data, 1, 1);
        let other_data = GameData::from_files([("BAR.DAT".to_string(), vec![9])]);
        let err = load(&bytes, &other_data).unwrap_err();
        assert_eq!(err, SaveError::DataFingerprintMismatch);
    }

    #[test]
    fn encode_then_load_round_trips() {
        let data = synthetic_data();
        let state = synthetic_state();
        let bytes = encode(&state, &data, 42, 7);
        let (header, loaded) = load(&bytes, &data).unwrap();
        assert_eq!(header.seed, 42);
        assert_eq!(header.tick_index, 7);
        assert_eq!(loaded.prng, state.prng);
    }

    #[test]
    fn data_fingerprint_is_stable_and_content_sensitive() {
        let a = synthetic_data();
        let b = synthetic_data();
        assert_eq!(data_fingerprint(&a), data_fingerprint(&b));
        let c = GameData::from_files([("FOO.DAT".to_string(), vec![1, 2, 4])]);
        assert_ne!(data_fingerprint(&a), data_fingerprint(&c));
    }

    /// D-SAVE1's collection-ordering invariant, exercised directly: the
    /// same populated `SaveState` serializes to byte-identical output on
    /// two independent calls (the in-process half of the guarantee — the
    /// committed golden hash, D-SAVE10, is the cross-machine half).
    #[test]
    fn serializing_a_populated_save_state_twice_is_byte_identical() {
        let mut state = synthetic_state();
        state.windows.raw_words.insert(0x4B10, 7);
        state.windows.raw_words.insert(0x4B02, 3);
        state.windows.raw_bytes.insert(0x10, 1);
        state
            .windows
            .raw_strings
            .insert(0x20, gbx_vm::VmString::from_bytes(b"hi".to_vec()));
        let bytes1 = postcard::to_allocvec(&state).unwrap();
        let bytes2 = postcard::to_allocvec(&state).unwrap();
        assert_eq!(bytes1, bytes2);
    }

    #[test]
    fn save_state_postcard_round_trips() {
        let state = synthetic_state();
        let bytes = postcard::to_allocvec(&state).unwrap();
        let restored: SaveState = postcard::from_bytes(&bytes).unwrap();
        let bytes2 = postcard::to_allocvec(&restored).unwrap();
        assert_eq!(bytes, bytes2);
    }
}
