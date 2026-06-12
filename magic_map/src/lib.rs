//! Declaration-site, fallible struct/enum mapping.
//!
//! `magic_map!` declares a mapping between two types as a standalone
//! statement — not as attributes on the types. Every destination field
//! without an explicit override is auto-filled from the same-named source
//! field through the [`MapFrom`] leaf funnel, so identities, `String↔Uuid`,
//! `Decimal↔f64`, `Option`/`Vec` wrappers, and previously-mapped enums and
//! structs compose for free — and every conversion that can lose information
//! is fallible and surfaces a [`MappingError`].
//!
//! Because the mapping lives at the call site, it also works when **both**
//! types are foreign or generated (prost protos, sqlx rows, OpenAPI DTOs):
//! the fn form sidesteps the orphan rule entirely, and the metadata derive
//! can be injected through codegen config (e.g. prost-build's
//! `type_attribute`).
//!
//! ```
//! use magic_map::{magic_map, MagicMap, MapInto};
//!
//! mod db {
//!     #[derive(magic_map::MagicMap)]
//!     pub struct User {
//!         pub id: String,
//!         pub name: String,
//!         pub age: i32,
//!     }
//! }
//!
//! mod dtos {
//!     #[derive(Debug, magic_map::MagicMap)]
//!     pub struct UserResponse {
//!         pub id: String,
//!         pub name: String,
//!         pub age: i64,    // i32 → i64 widens losslessly, so it automaps
//!         pub vip: bool,   // absent from the source → explicit override
//!     }
//! }
//!
//! magic_map!(db::User => dtos::UserResponse {
//!     vip: src.age > 90,
//! });
//!
//! let dto: dtos::UserResponse = db::User {
//!     id: "u1".into(),
//!     name: "Ada".into(),
//!     age: 36,
//! }
//! .map_into()
//! .unwrap();
//! assert_eq!(dto.age, 36);
//! assert!(!dto.vip);
//! ```
//!
//! See the [README](https://github.com/rsansores/magic_map) for the full
//! grammar tour: fn form, tuple sources, enum mappings with variant renames,
//! the `..Default::default()` optionality adaptor, and the prost integration
//! recipe.
//!
//! # Features
//!
//! Leaf conversions for third-party types are opt-in:
//!
//! | feature   | leaves                                                           |
//! |-----------|------------------------------------------------------------------|
//! | `uuid`    | `Uuid` identity, `String↔Uuid` (strict parse)                     |
//! | `chrono`  | date/time identities, `DateTime<Utc>↔String` (rfc3339), `NaiveDate↔String` (ISO-8601) |
//! | `decimal` | `Decimal` identity, `Decimal↔f64`/`String` (strict, no NaN/∞)     |
//! | `json`    | `serde_json::Value` identity                                     |
//! | `full`    | all of the above                                                 |
//!
//! Leaves for your **own** types are declared with [`map_identity!`],
//! [`map_display!`], [`map_parse!`], or a plain `MapFrom` impl in the crate
//! that owns the type.

use std::error::Error;
use std::fmt;

pub use magic_map_macros::{magic_map, MagicMap};

#[doc(hidden)]
pub use magic_map_macros::__magic_map_expand;

/// The single error surfaced by every mapping. Layer error types convert from
/// it once (e.g. `ApiError: From<MappingError>`), so call sites just bubble
/// with `?`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingError {
    InvalidUuid { field: &'static str },
    OutOfRange { field: &'static str },
    Parse { field: &'static str },
    Missing { field: &'static str },
    Custom(String),
}

impl fmt::Display for MappingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MappingError::InvalidUuid { field } => write!(f, "invalid uuid in `{field}`"),
            MappingError::OutOfRange { field } => write!(f, "value out of range in `{field}`"),
            MappingError::Parse { field } => write!(f, "parse error in `{field}`"),
            MappingError::Missing { field } => write!(f, "missing value for `{field}`"),
            MappingError::Custom(m) => write!(f, "{m}"),
        }
    }
}
impl Error for MappingError {}

/// Fallible field/struct/enum conversion. Implemented by `magic_map!` for
/// structs and enums, and by the leaf impls below for known type pairs.
///
/// Orphan-rule note: a `MapFrom<Src> for Dest` impl is only legal in a crate
/// that owns `Dest` or `Src`. Mappings where one side is local use the impl
/// form of `magic_map!`. Foreign→foreign mappings (e.g. db→proto in a neutral
/// service crate) cannot carry a trait impl at all — use the fn form
/// (`magic_map!(pub fn name: Src => Dest)`), which still reuses the leaf
/// conversions.
pub trait MapFrom<S>: Sized {
    fn map_from(src: S) -> Result<Self, MappingError>;
}

/// Call-side ergonomics: `let dto: Dto = db.map_into()?;`
pub trait MapInto<D> {
    fn map_into(self) -> Result<D, MappingError>;
}
impl<S, D: MapFrom<S>> MapInto<D> for S {
    fn map_into(self) -> Result<D, MappingError> {
        D::map_from(self)
    }
}

/// Identity conversions for known leaf types. Deliberately NOT a blanket
/// `impl<T> MapFrom<T> for T` — that overlaps the `Option`/`Vec` wrappers and
/// fails coherence. Add a new leaf in one line.
macro_rules! leaf_identity {
    ($($t:ty),* $(,)?) => {$(
        impl MapFrom<$t> for $t {
            fn map_from(src: $t) -> Result<Self, MappingError> { Ok(src) }
        }
    )*};
}
leaf_identity!(
    bool, char, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64, String,
);
#[cfg(feature = "uuid")]
leaf_identity!(uuid::Uuid);
#[cfg(feature = "decimal")]
leaf_identity!(rust_decimal::Decimal);
#[cfg(feature = "chrono")]
leaf_identity!(
    chrono::DateTime<chrono::Utc>,
    chrono::NaiveDate,
    chrono::NaiveDateTime,
    chrono::NaiveTime,
);
#[cfg(feature = "json")]
leaf_identity!(serde_json::Value);

/// Lossless numeric widenings automap (the conversion carries no decision);
/// narrowing stays an explicit `as` cast in the mapper, where the lossiness
/// is visible.
macro_rules! leaf_widen {
    ($($s:ty => $d:ty),+ $(,)?) => {$(
        impl MapFrom<$s> for $d {
            fn map_from(src: $s) -> Result<Self, MappingError> {
                Ok(<$d>::from(src))
            }
        }
    )+};
}
leaf_widen!(
    u8 => u16, u8 => u32, u8 => u64, u8 => i16, u8 => i32, u8 => i64,
    u16 => u32, u16 => u64, u16 => i32, u16 => i64,
    u32 => u64, u32 => i64,
    i8 => i16, i8 => i32, i8 => i64,
    i16 => i32, i16 => i64,
    i32 => i64,
    f32 => f64,
);

impl<S, D: MapFrom<S>> MapFrom<Option<S>> for Option<D> {
    fn map_from(src: Option<S>) -> Result<Self, MappingError> {
        match src {
            Some(s) => Ok(Some(D::map_from(s)?)),
            None => Ok(None),
        }
    }
}
impl<S, D: MapFrom<S>> MapFrom<Vec<S>> for Vec<D> {
    fn map_from(src: Vec<S>) -> Result<Self, MappingError> {
        src.into_iter().map(D::map_from).collect()
    }
}

// ── known cross-type leaf conversions (written once) ──────────────────────────

#[cfg(feature = "uuid")]
mod uuid_leaves {
    use super::{MapFrom, MappingError};
    use uuid::Uuid;

    impl MapFrom<String> for Uuid {
        fn map_from(src: String) -> Result<Self, MappingError> {
            Uuid::parse_str(&src).map_err(|_| MappingError::InvalidUuid { field: "<uuid>" })
        }
    }
    impl MapFrom<Uuid> for String {
        fn map_from(src: Uuid) -> Result<Self, MappingError> {
            Ok(src.to_string())
        }
    }
}

#[cfg(feature = "decimal")]
mod decimal_leaves {
    use super::{MapFrom, MappingError};
    use rust_decimal::prelude::ToPrimitive;
    use rust_decimal::Decimal;

    impl MapFrom<Decimal> for f64 {
        fn map_from(src: Decimal) -> Result<Self, MappingError> {
            src.to_f64()
                .ok_or(MappingError::OutOfRange { field: "<decimal>" })
        }
    }
    impl MapFrom<f64> for Decimal {
        fn map_from(src: f64) -> Result<Self, MappingError> {
            // NaN/±inf error out rather than silently dropping the value; JSON
            // can't carry them anyway, so API paths never hit this.
            Decimal::from_f64_retain(src).ok_or(MappingError::OutOfRange { field: "<decimal>" })
        }
    }
    impl MapFrom<String> for Decimal {
        fn map_from(src: String) -> Result<Self, MappingError> {
            src.parse()
                .map_err(|_| MappingError::Parse { field: "<decimal>" })
        }
    }
    impl MapFrom<Decimal> for String {
        fn map_from(src: Decimal) -> Result<Self, MappingError> {
            Ok(src.to_string())
        }
    }
}

#[cfg(feature = "chrono")]
mod chrono_leaves {
    use super::{MapFrom, MappingError};
    use chrono::{DateTime, NaiveDate, Utc};

    /// Canonical wire format for timestamps is rfc3339.
    impl MapFrom<String> for DateTime<Utc> {
        fn map_from(src: String) -> Result<Self, MappingError> {
            DateTime::parse_from_rfc3339(&src)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|_| MappingError::Parse {
                    field: "<datetime>",
                })
        }
    }
    impl MapFrom<DateTime<Utc>> for String {
        fn map_from(src: DateTime<Utc>) -> Result<Self, MappingError> {
            Ok(src.to_rfc3339())
        }
    }

    // Dates cross the wire as ISO-8601 (`YYYY-MM-DD`) strings — `NaiveDate`'s
    // canonical `Display`/`FromStr` form (strict on the way in).
    crate::map_display!(NaiveDate);
    crate::map_parse!(NaiveDate);
}

// ── `..Default::default()` field machinery (used by magic_map! codegen) ──────
//
// With the defaults trailer, every non-overridden field compiles to
// `(&mut &mut &mut MapPair(Some(src.f), Some(fb.f))).map_field_or()`. Method
// probing walks three tiers by autoderef — the macro never needs to know
// field types:
//   1. `Option<S>` source → required dest: funnel the inner value; `None`
//      falls back to the default instance's field (declared on the model).
//   2. plain funnel (covers `Option → Option`, so `None → None` stays
//      Rust-like — an optional dest never gets a value invented).
//   3. plain source → `Option<U>` dest: funnel and wrap in `Some` ("set").
//      NOTE: if `None` means "don't touch" on your update/patch models,
//      keep explicit `Some(...)` wraps in those mappers instead.

#[doc(hidden)]
pub struct MapPair<S, D>(pub Option<S>, pub Option<D>);

#[doc(hidden)]
pub trait MapFieldOpt<D> {
    fn map_field_or(self) -> Result<D, MappingError>;
}
impl<S, D: MapFrom<S>> MapFieldOpt<D> for &mut &mut &mut MapPair<Option<S>, D> {
    fn map_field_or(self) -> Result<D, MappingError> {
        match self.0.take().expect("magic_map field consumed twice") {
            Some(s) => D::map_from(s),
            None => Ok(self.1.take().expect("magic_map fallback consumed twice")),
        }
    }
}

#[doc(hidden)]
pub trait MapFieldVal<D> {
    fn map_field_or(self) -> Result<D, MappingError>;
}
impl<S, D: MapFrom<S>> MapFieldVal<D> for &mut &mut MapPair<S, D> {
    fn map_field_or(self) -> Result<D, MappingError> {
        D::map_from(self.0.take().expect("magic_map field consumed twice"))
    }
}

#[doc(hidden)]
pub trait MapFieldWrap<D> {
    fn map_field_or(self) -> Result<D, MappingError>;
}
impl<S, U: MapFrom<S>> MapFieldWrap<Option<U>> for &mut MapPair<S, Option<U>> {
    fn map_field_or(self) -> Result<Option<U>, MappingError> {
        let src = self.0.take().expect("magic_map field consumed twice");
        Ok(Some(U::map_from(src)?))
    }
}

/// `map_identity!(MyEnum);` — `MapFrom<MyEnum> for MyEnum`, so same-typed
/// fields automap (model→model moves, e.g. invite→update). Declare next to
/// the type; the orphan rule keeps it in the owning crate.
#[macro_export]
macro_rules! map_identity {
    ($($t:ty),+ $(,)?) => {$(
        impl $crate::MapFrom<$t> for $t {
            fn map_from(src: $t) -> ::core::result::Result<Self, $crate::MappingError> {
                Ok(src)
            }
        }
    )+};
}

/// `map_display!(MyEnum);` — `MapFrom<MyEnum> for String` via `Display`, so
/// enum→string fields automap (pairs with strum's `Display` derive).
#[macro_export]
macro_rules! map_display {
    ($($t:ty),+ $(,)?) => {$(
        impl $crate::MapFrom<$t> for ::std::string::String {
            fn map_from(src: $t) -> ::core::result::Result<Self, $crate::MappingError> {
                Ok(src.to_string())
            }
        }
    )+};
}

/// `map_parse!(MyEnum);` — `MapFrom<String> for MyEnum` via `FromStr`, so
/// string→enum fields automap strictly (pairs with strum's `EnumString`).
#[macro_export]
macro_rules! map_parse {
    ($($t:ty),+ $(,)?) => {$(
        impl $crate::MapFrom<::std::string::String> for $t {
            fn map_from(src: ::std::string::String) -> ::core::result::Result<Self, $crate::MappingError> {
                src.parse().map_err(|_| $crate::MappingError::Parse {
                    field: ::core::stringify!($t),
                })
            }
        }
    )+};
}
