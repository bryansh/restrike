//! Element typing and the domain grammar (`docs/design/rules-packs.md`
//! D-RP3a's `records` shape: `"min..=max"` / `"set(0,1,2,4)"` /
//! `"bitmask(n)"`).

/// A stored value's on-disk width and signedness, per D-RP2 ("packs store
/// the runtime image's encoding verbatim"). Every anchored table's byte
/// comparison in the verify engine goes through [`ElementType::to_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementType {
    U8,
    I8,
    U16Le,
    I16Le,
    U32Le,
    I32Le,
}

/// [`ElementType::parse`] / [`Domain::parse`]'s failure mode — always a
/// pack-authoring bug, surfaced through [`super::SchemaError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementParseError(pub String);

impl ElementType {
    pub fn parse(s: &str) -> Result<Self, ElementParseError> {
        match s {
            "u8" => Ok(ElementType::U8),
            "i8" => Ok(ElementType::I8),
            "u16le" => Ok(ElementType::U16Le),
            "i16le" => Ok(ElementType::I16Le),
            "u32le" => Ok(ElementType::U32Le),
            "i32le" => Ok(ElementType::I32Le),
            other => Err(ElementParseError(format!(
                "unknown element type '{other}' (expected one of u8, i8, u16le, i16le, u32le, i32le)"
            ))),
        }
    }

    /// On-disk width in bytes — the unit D-RP7's anchor-length formula
    /// (`len == product(axes) x element width`) multiplies against.
    pub fn width(&self) -> usize {
        match self {
            ElementType::U8 | ElementType::I8 => 1,
            ElementType::U16Le | ElementType::I16Le => 2,
            ElementType::U32Le | ElementType::I32Le => 4,
        }
    }

    /// The inclusive range a value of this type can represent, for domain
    /// bounds-checking a stored (pre-widening) `i64`.
    pub fn native_range(&self) -> (i64, i64) {
        match self {
            ElementType::U8 => (u8::MIN as i64, u8::MAX as i64),
            ElementType::I8 => (i8::MIN as i64, i8::MAX as i64),
            ElementType::U16Le => (u16::MIN as i64, u16::MAX as i64),
            ElementType::I16Le => (i16::MIN as i64, i16::MAX as i64),
            ElementType::U32Le => (u32::MIN as i64, u32::MAX as i64),
            ElementType::I32Le => (i32::MIN as i64, i32::MAX as i64),
        }
    }

    /// Serializes `value` to this element's on-disk byte width, little-endian
    /// (every element type this schema declares is little-endian — see
    /// D-RP2's "packs store the runtime image's encoding verbatim").
    /// Returns `None` if `value` doesn't fit — an out-of-range table cell,
    /// which schema validation (`pack::validate`) is expected to have
    /// already rejected before this is ever called for real.
    pub fn to_bytes(&self, value: i64) -> Option<Vec<u8>> {
        let (min, max) = self.native_range();
        if value < min || value > max {
            return None;
        }
        Some(match self {
            ElementType::U8 => vec![value as u8],
            ElementType::I8 => vec![value as i8 as u8],
            ElementType::U16Le => (value as u16).to_le_bytes().to_vec(),
            ElementType::I16Le => (value as i16).to_le_bytes().to_vec(),
            ElementType::U32Le => (value as u32).to_le_bytes().to_vec(),
            ElementType::I32Le => (value as i32).to_le_bytes().to_vec(),
        })
    }
}

/// A `records`-shape column's declared value domain (D-RP3a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Domain {
    /// `"min..=max"`, inclusive both ends.
    Range { min: i64, max: i64 },
    /// `"set(0,1,2,4)"` — a non-contiguous enumeration of legal values.
    Set(Vec<i64>),
    /// `"bitmask(n)"` — any combination of the low `n` bits (`0..=2^n - 1`).
    Bitmask(u32),
}

impl Domain {
    pub fn parse(s: &str) -> Result<Self, ElementParseError> {
        let s = s.trim();
        if let Some(inner) = s.strip_prefix("set(").and_then(|r| r.strip_suffix(')')) {
            let values = inner
                .split(',')
                .map(|piece| {
                    piece.trim().parse::<i64>().map_err(|_| {
                        ElementParseError(format!(
                            "invalid set(...) member '{piece}' in domain '{s}'"
                        ))
                    })
                })
                .collect::<Result<Vec<i64>, _>>()?;
            if values.is_empty() {
                return Err(ElementParseError(format!("empty set(...) domain '{s}'")));
            }
            return Ok(Domain::Set(values));
        }
        if let Some(inner) = s.strip_prefix("bitmask(").and_then(|r| r.strip_suffix(')')) {
            let n: u32 = inner
                .trim()
                .parse()
                .map_err(|_| ElementParseError(format!("invalid bitmask(n) domain '{s}'")))?;
            if n == 0 || n > 63 {
                return Err(ElementParseError(format!(
                    "bitmask(n) domain '{s}' must have 1 <= n <= 63"
                )));
            }
            return Ok(Domain::Bitmask(n));
        }
        if let Some((min_s, max_s)) = s.split_once("..=") {
            let min = min_s
                .trim()
                .parse::<i64>()
                .map_err(|_| ElementParseError(format!("invalid range domain '{s}'")))?;
            let max = max_s
                .trim()
                .parse::<i64>()
                .map_err(|_| ElementParseError(format!("invalid range domain '{s}'")))?;
            if min > max {
                return Err(ElementParseError(format!(
                    "range domain '{s}' has min > max"
                )));
            }
            return Ok(Domain::Range { min, max });
        }
        Err(ElementParseError(format!(
            "domain '{s}' matches none of min..=max, set(...), bitmask(n)"
        )))
    }

    pub fn contains(&self, value: i64) -> bool {
        match self {
            Domain::Range { min, max } => value >= *min && value <= *max,
            Domain::Set(values) => values.contains(&value),
            Domain::Bitmask(n) => {
                let max = (1i64 << *n) - 1;
                (0..=max).contains(&value)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_widths() {
        assert_eq!(ElementType::U8.width(), 1);
        assert_eq!(ElementType::I8.width(), 1);
        assert_eq!(ElementType::U16Le.width(), 2);
        assert_eq!(ElementType::I16Le.width(), 2);
        assert_eq!(ElementType::U32Le.width(), 4);
        assert_eq!(ElementType::I32Le.width(), 4);
    }

    #[test]
    fn element_parse_rejects_unknown() {
        assert!(ElementType::parse("u64le").is_err());
    }

    #[test]
    fn to_bytes_little_endian() {
        assert_eq!(
            ElementType::U16Le.to_bytes(0x0c08).unwrap(),
            vec![0x08, 0x0c]
        );
        assert_eq!(
            ElementType::I32Le.to_bytes(1501).unwrap(),
            1501i32.to_le_bytes().to_vec()
        );
    }

    #[test]
    fn to_bytes_rejects_out_of_range() {
        assert!(ElementType::U8.to_bytes(256).is_none());
        assert!(ElementType::U8.to_bytes(-1).is_none());
    }

    #[test]
    fn domain_range_parses_and_contains() {
        let d = Domain::parse("0..=5000").unwrap();
        assert_eq!(d, Domain::Range { min: 0, max: 5000 });
        assert!(d.contains(0));
        assert!(d.contains(5000));
        assert!(!d.contains(5001));
        assert!(!d.contains(-1));
    }

    #[test]
    fn domain_set_parses_and_contains() {
        let d = Domain::parse("set(0,1,2,4)").unwrap();
        assert_eq!(d, Domain::Set(vec![0, 1, 2, 4]));
        assert!(d.contains(4));
        assert!(!d.contains(3));
    }

    #[test]
    fn domain_bitmask_parses_and_contains() {
        let d = Domain::parse("bitmask(8)").unwrap();
        assert_eq!(d, Domain::Bitmask(8));
        assert!(d.contains(0));
        assert!(d.contains(255));
        assert!(!d.contains(256));
    }

    #[test]
    fn domain_rejects_garbage() {
        assert!(Domain::parse("banana").is_err());
        assert!(Domain::parse("5..=1").is_err());
        assert!(Domain::parse("set()").is_err());
    }
}
