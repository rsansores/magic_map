//! Declaration-site, fallible struct/enum mapping.
//!
//! `magic_map!` declares a mapping between two types as a standalone
//! statement — not as attributes on the types. Every destination field
//! without an explicit override is auto-filled from the same-named source
//! field through the [`TryMapFrom`] leaf funnel, so identities, `String↔Uuid`,
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
//! A crate using the fn form calls [`magic_map_scope!`] once in its crate
//! root. That plants the crate-local funnel the generated functions resolve
//! nested fields through, so foreign→foreign mappings compose with each other
//! the same way impl-form ones do — see the macro's docs for the whole story,
//! including why custom leaves have to be named there.
//!
//! ```
//! use magic_map::{magic_map, MagicMap, TryMapInto};
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
//! .try_map_into()
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
//! | feature    | leaves / behavior                                                        |
//! |------------|--------------------------------------------------------------------------|
//! | `uuid`     | `Uuid` identity, `String↔Uuid` (strict parse)                             |
//! | `chrono`   | date/time identities, `DateTime<Utc>↔String` (rfc3339), `NaiveDate↔String` (ISO-8601) |
//! | `decimal`  | `Decimal` identity, `Decimal↔f64`/`String` (strict, no NaN/∞)             |
//! | `json`     | `serde_json::Value` identity                                             |
//! | `validate` | `MappingError::Validation` variant; auto-validates destinations with `#[validate(...)]` fields |
//! | `full`     | all of the above                                                         |
//!
//! Leaves for your **own** types are declared with [`map_identity!`],
//! [`map_display!`], [`map_parse!`], or a plain `TryMapFrom` impl in the crate
//! that owns the type.

use std::error::Error;
use std::fmt;

pub use magic_map_macros::{magic_map, magic_map_leaves, MagicMap, mapped};

#[doc(hidden)]
pub use magic_map_macros::__magic_map_expand;

// The scope leaf groups name third-party leaf types through these, so a crate
// calling `magic_map_scope!` does not need uuid/chrono/decimal/json as direct
// dependencies just because this crate's features enable those leaves.
#[doc(hidden)]
pub mod __rx {
    #[cfg(feature = "chrono")]
    pub use chrono;
    #[cfg(feature = "decimal")]
    pub use rust_decimal;
    #[cfg(feature = "json")]
    pub use serde_json;
    #[cfg(feature = "uuid")]
    pub use uuid;
}

// Re-exported so generated code can reach the `Validate` trait as
// `::magic_map::validator::Validate` without the call-site (or a neutral mapper
// crate) needing a direct `validator` dependency, and so the `validator` version
// that backs `MappingError::Validation` is the same one the generated
// `.validate()` call resolves against.
#[cfg(feature = "validate")]
#[doc(hidden)]
pub use validator;

/// The single error surfaced by every mapping. Layer error types convert from
/// it once (e.g. `ApiError: From<MappingError>`), so call sites just bubble
/// with `?`.
///
/// `Validation` is only present when the `validate` feature is enabled and the
/// destination struct has `#[validate(...)]` field annotations.
///
/// Note: enabling `validate` drops the `Eq` impl, because
/// `validator::ValidationErrors` is only `PartialEq`. Cargo features are
/// additive, so this applies build-wide once *any* crate in the graph turns the
/// feature on — `==`, `match`, and `assert_eq!` still work; an `Eq` bound does
/// not.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(feature = "validate"), derive(Eq))]
pub enum MappingError {
    InvalidUuid {
        field: &'static str,
    },
    OutOfRange {
        field: &'static str,
    },
    Parse {
        field: &'static str,
    },
    Missing {
        field: &'static str,
    },
    Custom(String),
    #[cfg(feature = "validate")]
    Validation(validator::ValidationErrors),
}

impl fmt::Display for MappingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MappingError::InvalidUuid { field } => write!(f, "invalid uuid in `{field}`"),
            MappingError::OutOfRange { field } => write!(f, "value out of range in `{field}`"),
            MappingError::Parse { field } => write!(f, "parse error in `{field}`"),
            MappingError::Missing { field } => write!(f, "missing value for `{field}`"),
            MappingError::Custom(m) => write!(f, "{m}"),
            #[cfg(feature = "validate")]
            MappingError::Validation(e) => write!(f, "validation failed: {e}"),
        }
    }
}
impl Error for MappingError {}

// `From` conversions for the leaf parse errors, so an override expression
// inside a `magic_map!` body can `?` them directly:
//
//     magic_map!(pub fn f: Src => Dest {
//         id: Uuid::parse_str(&src.raw)?,   // uuid::Error -> MappingError
//     });
//
// The auto-path already converts these types (String -> Uuid, etc.); these
// impls make the same leaves reachable from a manual override, instead of
// hand-rolling `.map_err(|e| MappingError::Custom(e.to_string()))`. They mirror
// the auto-path's behaviour: a structured variant, the source message dropped.
// `field` is `"<override>"` because a `From` impl has no field-name context.
#[cfg(feature = "uuid")]
impl From<uuid::Error> for MappingError {
    fn from(_: uuid::Error) -> Self {
        MappingError::InvalidUuid {
            field: "<override>",
        }
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::ParseError> for MappingError {
    fn from(_: chrono::ParseError) -> Self {
        MappingError::Parse {
            field: "<override>",
        }
    }
}

#[cfg(feature = "decimal")]
impl From<rust_decimal::Error> for MappingError {
    fn from(_: rust_decimal::Error) -> Self {
        MappingError::Parse {
            field: "<override>",
        }
    }
}

/// Fallible field/struct/enum conversion. Implemented by `magic_map!` for
/// structs and enums, and by the leaf impls below for known type pairs.
///
/// Orphan-rule note: a `TryMapFrom<Src> for Dest` impl is only legal in a crate
/// that owns `Dest` or `Src`. Mappings where one side is local use the impl
/// form of `magic_map!`. Foreign→foreign mappings (e.g. db→proto in a neutral
/// service crate) cannot carry a trait impl at all — use the fn form
/// (`magic_map!(pub fn name: Src => Dest)`), which still reuses the leaf
/// conversions.
pub trait TryMapFrom<S>: Sized {
    fn try_map_from(src: S) -> Result<Self, MappingError>;
}

/// Call-side ergonomics: `let dto: Dto = db.try_map_into()?;`
pub trait TryMapInto<D> {
    fn try_map_into(self) -> Result<D, MappingError>;
}
impl<S, D: TryMapFrom<S>> TryMapInto<D> for S {
    fn try_map_into(self) -> Result<D, MappingError> {
        D::try_map_from(self)
    }
}

/// Infallible struct/enum conversion — the half of the funnel that carries no
/// decision. A mapping is infallible when every field pair is: an identity, a
/// lossless widening, or another infallible mapping. `String` → `Uuid` is not,
/// and that asymmetry is the whole point — a conversion that can fail says so
/// in its type, and one that cannot does not make every call site pretend.
///
/// `magic_map!(infallible ...)` emits this. The check is structural rather
/// than declarative: the expansion has no `?` in it, so a field pair that only
/// has `TryMapFrom` fails to resolve. You cannot claim infallible wrongly.
pub trait MapFrom<S>: Sized {
    fn map_from(src: S) -> Self;
}

/// Call-side ergonomics: `let dto: Dto = db.map_into();` — no `?`.
pub trait MapInto<D> {
    fn map_into(self) -> D;
}
impl<S, D: MapFrom<S>> MapInto<D> for S {
    fn map_into(self) -> D {
        D::map_from(self)
    }
}

/// Identity conversions for known leaf types. Deliberately NOT a blanket
/// `impl<T> TryMapFrom<T> for T` — that overlaps the `Option`/`Vec` wrappers and
/// fails coherence. Add a new leaf in one line.
macro_rules! leaf_identity {
    ($($t:ty),* $(,)?) => {$(
        impl TryMapFrom<$t> for $t {
            fn try_map_from(src: $t) -> Result<Self, MappingError> { Ok(src) }
        }
        impl MapFrom<$t> for $t {
            fn map_from(src: $t) -> Self { src }
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
        impl TryMapFrom<$s> for $d {
            fn try_map_from(src: $s) -> Result<Self, MappingError> {
                Ok(<$d>::from(src))
            }
        }
        impl MapFrom<$s> for $d {
            fn map_from(src: $s) -> Self { <$d>::from(src) }
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

impl<S, D: TryMapFrom<S>> TryMapFrom<Option<S>> for Option<D> {
    fn try_map_from(src: Option<S>) -> Result<Self, MappingError> {
        match src {
            Some(s) => Ok(Some(D::try_map_from(s)?)),
            None => Ok(None),
        }
    }
}
impl<S, D: TryMapFrom<S>> TryMapFrom<Vec<S>> for Vec<D> {
    fn try_map_from(src: Vec<S>) -> Result<Self, MappingError> {
        src.into_iter().map(D::try_map_from).collect()
    }
}

impl<S, D: MapFrom<S>> MapFrom<Option<S>> for Option<D> {
    fn map_from(src: Option<S>) -> Self {
        src.map(D::map_from)
    }
}
impl<S, D: MapFrom<S>> MapFrom<Vec<S>> for Vec<D> {
    fn map_from(src: Vec<S>) -> Self {
        src.into_iter().map(D::map_from).collect()
    }
}

// ── known cross-type leaf conversions (written once) ──────────────────────────

#[cfg(feature = "uuid")]
mod uuid_leaves {
    use super::{MapFrom, TryMapFrom, MappingError};
    use uuid::Uuid;

    impl TryMapFrom<String> for Uuid {
        fn try_map_from(src: String) -> Result<Self, MappingError> {
            Uuid::parse_str(&src).map_err(|_| MappingError::InvalidUuid { field: "<uuid>" })
        }
    }
    impl TryMapFrom<Uuid> for String {
        fn try_map_from(src: Uuid) -> Result<Self, MappingError> {
            Ok(src.to_string())
        }
    }
    impl MapFrom<Uuid> for String {
        fn map_from(src: Uuid) -> Self {
            src.to_string()
        }
    }
}

#[cfg(feature = "decimal")]
mod decimal_leaves {
    use super::{MapFrom, TryMapFrom, MappingError};
    use rust_decimal::prelude::ToPrimitive;
    use rust_decimal::Decimal;

    impl TryMapFrom<Decimal> for f64 {
        fn try_map_from(src: Decimal) -> Result<Self, MappingError> {
            src.to_f64()
                .ok_or(MappingError::OutOfRange { field: "<decimal>" })
        }
    }
    impl TryMapFrom<f64> for Decimal {
        fn try_map_from(src: f64) -> Result<Self, MappingError> {
            // NaN/±inf error out rather than silently dropping the value; JSON
            // can't carry them anyway, so API paths never hit this.
            Decimal::from_f64_retain(src).ok_or(MappingError::OutOfRange { field: "<decimal>" })
        }
    }
    impl TryMapFrom<String> for Decimal {
        fn try_map_from(src: String) -> Result<Self, MappingError> {
            src.parse()
                .map_err(|_| MappingError::Parse { field: "<decimal>" })
        }
    }
    impl TryMapFrom<Decimal> for String {
        fn try_map_from(src: Decimal) -> Result<Self, MappingError> {
            Ok(src.to_string())
        }
    }
    impl MapFrom<Decimal> for String {
        fn map_from(src: Decimal) -> Self {
            src.to_string()
        }
    }
}

#[cfg(feature = "chrono")]
mod chrono_leaves {
    use super::{TryMapFrom, MappingError};
    use chrono::{DateTime, NaiveDate, Utc};

    /// Canonical wire format for timestamps is rfc3339.
    impl TryMapFrom<String> for DateTime<Utc> {
        fn try_map_from(src: String) -> Result<Self, MappingError> {
            DateTime::parse_from_rfc3339(&src)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|_| MappingError::Parse {
                    field: "<datetime>",
                })
        }
    }
    impl TryMapFrom<DateTime<Utc>> for String {
        fn try_map_from(src: DateTime<Utc>) -> Result<Self, MappingError> {
            Ok(src.to_rfc3339())
        }
    }
    impl super::MapFrom<DateTime<Utc>> for String {
        fn map_from(src: DateTime<Utc>) -> Self {
            src.to_rfc3339()
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
impl<S, D: TryMapFrom<S>> MapFieldOpt<D> for &mut &mut &mut MapPair<Option<S>, D> {
    fn map_field_or(self) -> Result<D, MappingError> {
        match self.0.take().expect("magic_map field consumed twice") {
            Some(s) => D::try_map_from(s),
            None => Ok(self.1.take().expect("magic_map fallback consumed twice")),
        }
    }
}

#[doc(hidden)]
pub trait MapFieldVal<D> {
    fn map_field_or(self) -> Result<D, MappingError>;
}
impl<S, D: TryMapFrom<S>> MapFieldVal<D> for &mut &mut MapPair<S, D> {
    fn map_field_or(self) -> Result<D, MappingError> {
        D::try_map_from(self.0.take().expect("magic_map field consumed twice"))
    }
}

#[doc(hidden)]
pub trait MapFieldWrap<D> {
    fn map_field_or(self) -> Result<D, MappingError>;
}
impl<S, U: TryMapFrom<S>> MapFieldWrap<Option<U>> for &mut MapPair<S, Option<U>> {
    fn map_field_or(self) -> Result<Option<U>, MappingError> {
        let src = self.0.take().expect("magic_map field consumed twice");
        Ok(Some(U::try_map_from(src)?))
    }
}

/// `map_identity!(MyEnum);` — `TryMapFrom<MyEnum> for MyEnum` plus its
/// infallible `MapFrom` twin (an identity cannot fail), so same-typed fields
/// automap in both funnels (model→model moves, e.g. invite→update). Declare
/// next to the type; the orphan rule keeps it in the owning crate.
#[macro_export]
macro_rules! map_identity {
    ($($t:ty),+ $(,)?) => {$(
        impl $crate::TryMapFrom<$t> for $t {
            fn try_map_from(src: $t) -> ::core::result::Result<Self, $crate::MappingError> {
                Ok(src)
            }
        }
        impl $crate::MapFrom<$t> for $t {
            fn map_from(src: $t) -> Self {
                src
            }
        }
    )+};
}

/// `map_display!(MyEnum);` — `TryMapFrom<MyEnum> for String` plus its
/// infallible `MapFrom` twin (`Display` cannot fail), so enum→string fields
/// automap in both funnels (pairs with strum's `Display` derive).
#[macro_export]
macro_rules! map_display {
    ($($t:ty),+ $(,)?) => {$(
        impl $crate::TryMapFrom<$t> for ::std::string::String {
            fn try_map_from(src: $t) -> ::core::result::Result<Self, $crate::MappingError> {
                Ok(src.to_string())
            }
        }
        impl $crate::MapFrom<$t> for ::std::string::String {
            fn map_from(src: $t) -> Self {
                src.to_string()
            }
        }
    )+};
}

/// `map_parse!(MyEnum);` — `TryMapFrom<String> for MyEnum` via `FromStr`, so
/// string→enum fields automap strictly (pairs with strum's `EnumString`).
#[macro_export]
macro_rules! map_parse {
    ($($t:ty),+ $(,)?) => {$(
        impl $crate::TryMapFrom<::std::string::String> for $t {
            fn try_map_from(src: ::std::string::String) -> ::core::result::Result<Self, $crate::MappingError> {
                src.parse().map_err(|_| $crate::MappingError::Parse {
                    field: ::core::stringify!($t),
                })
            }
        }
    )+};
}

// ── Crate-local funnel for foreign→foreign mappings ─────────────────────────
//
// The fn form exists because `impl TryMapFrom<Src> for Dest` is only legal in a
// crate owning one of the two types. But a mapping's *fields* funnelled through
// `TryMapFrom` too, so a nested field whose own mapping was also foreign→foreign
// had nothing to resolve against: `Vec<Address>` → `Vec<AddressResponse>` needs
// `AddressResponse: TryMapFrom<Address>`, and that impl cannot exist anywhere.
//
// `magic_map_scope!` plants a trait in the *calling* crate. The orphan rule is
// satisfied by a local trait just as well as by a local type, so
// `impl LocalMapFrom<Address> for AddressResponse` is legal there even though
// both types are foreign, and the fn form emits one per mapping it declares.
//
// The trait is a CLOSED WORLD: everything the fn form funnels through has a
// `LocalMapFrom` impl, leaves included. Two dead ends forced that, both worth
// knowing before anyone tries to "simplify" this:
//
//   * A blanket bridge `impl<S, D: TryMapFrom<S>> LocalMapFrom<S> for D` overlaps
//     the per-pair impls and coherence rejects it — "upstream crates may add a
//     new impl of `TryMapFrom<Address>` for `AddressResponse` in future versions".
//
//   * Nor can a second, `TryMapFrom`-backed tier sit underneath to catch leaves.
//     Autoref tiering needs the tiers told apart by receiver SHAPE (as
//     `MapFieldOpt`/`MapFieldVal`/`MapFieldWrap` are); a tier separated only by
//     a where-bound hard-errors rather than falling through. A concrete
//     per-pair tier is worse: with the destination still open, probing matches
//     on the source alone and unifies the destination to whatever that impl
//     produces, so a source carrying both a declared mapping and a leaf — an
//     enum with a DTO twin *and* a `map_display!` to `String` — resolves to the
//     wrong one, and the expected type does not override it.
//
// So leaves are delegated in, and `magic_map_leaves!` keeps that from becoming
// per-consumer bookkeeping: a crate declares its leaves once and
// `leaves_from: [that_crate]` replays the list.

/// Declares the crate-local mapping funnel that the fn form of `magic_map!`
/// resolves nested fields through. Call it **once, in the crate root**
/// (`lib.rs` / `main.rs`) of any crate that declares a fn-form mapping.
///
/// ```ignore
/// // src/lib.rs
/// magic_map::magic_map_scope! {
///     leaves_from: [quickedge_commons, quickedge_db],
/// }
/// ```
///
/// Needed only for the fn form. A crate whose mappings are all impl form
/// (`magic_map!(Src => Dest)`, where one side is local) never calls it — those
/// funnel through `TryMapFrom` as they always have. Missing it reads as
/// ``could not find `__magic_map_scope` in the crate root``.
///
/// # Reaching your leaves
///
/// Built-in leaves (primitives, `String`, `Uuid`, `chrono`, `Decimal`,
/// `serde_json::Value`, the integer widenings) are always present. Your own
/// arrive through `leaves_from`, which replays the [`magic_map_leaves!`] block
/// of every crate named — so adding an enum there needs no edit in any
/// consumer.
///
/// `leaves: [..]` is the escape hatch for a one-off pair whose crate has no
/// block: a bare type is its identity, `Src => Dest` delegates that direction.
/// A generic wrapper goes in `generic_leaves`, `;`-separated so the `where`
/// clause's commas stay unambiguous:
///
/// ```ignore
/// magic_map::magic_map_scope! {
///     leaves_from: [quickedge_db],
///     leaves: [Celsius, Celsius => String],
///     generic_leaves: {
///         <S, D> Patch<S> => Patch<D> where D: ::magic_map::TryMapFrom<S>;
///     },
/// }
/// ```
///
/// A pair that never arrives fails at the mapping that needed it, naming both
/// types: ``the trait bound `String: LocalMapFrom<Celsius>` is not satisfied``.
#[macro_export]
macro_rules! magic_map_scope {
    () => { $crate::magic_map_scope!(from: [], leaves: [], generic_leaves: {}); };
    (from: [ $($from:ident),* $(,)? ] $(,)?) => {
        $crate::magic_map_scope!(from: [ $($from),* ], leaves: [], generic_leaves: {});
    };
    (leaves: [ $($leaf:tt)* ] $(,)?) => {
        $crate::magic_map_scope!(from: [], leaves: [ $($leaf)* ], generic_leaves: {});
    };
    (from: [ $($from:ident),* $(,)? ], leaves: [ $($leaf:tt)* ] $(,)?) => {
        $crate::magic_map_scope!(from: [ $($from),* ], leaves: [ $($leaf)* ], generic_leaves: {});
    };
    (from: [ $($from:ident),* $(,)? ], generic_leaves: { $($g:tt)* } $(,)?) => {
        $crate::magic_map_scope!(from: [ $($from),* ], leaves: [], generic_leaves: { $($g)* });
    };
    (
        from: [ $($from:ident),* $(,)? ],
        leaves: [ $($leaf:tt)* ],
        generic_leaves: { $($generic:tt)* } $(,)?
    ) => {
        #[doc(hidden)]
        pub mod __magic_map_scope {
            //! Generated by `magic_map::magic_map_scope!`. The fn form of
            //! `magic_map!` resolves its fields through `LocalMapFrom` here.

            #[allow(unused_imports)]
            use super::*;

            /// Crate-local twin of [`magic_map::TryMapFrom`]. The fn form
            /// implements it for pairs the orphan rule keeps off `TryMapFrom`;
            /// leaves are delegated in below.
            pub trait LocalTryMapFrom<S>: Sized {
                fn local_try_map_from(
                    src: S,
                ) -> ::core::result::Result<Self, $crate::MappingError>;
            }

            impl<S, D: LocalTryMapFrom<S>> LocalTryMapFrom<::core::option::Option<S>>
                for ::core::option::Option<D>
            {
                fn local_try_map_from(
                    src: ::core::option::Option<S>,
                ) -> ::core::result::Result<Self, $crate::MappingError> {
                    match src {
                        ::core::option::Option::Some(s) => {
                            ::core::result::Result::Ok(::core::option::Option::Some(
                                D::local_try_map_from(s)?,
                            ))
                        }
                        ::core::option::Option::None => {
                            ::core::result::Result::Ok(::core::option::Option::None)
                        }
                    }
                }
            }

            impl<S, D: LocalTryMapFrom<S>> LocalTryMapFrom<::std::vec::Vec<S>>
                for ::std::vec::Vec<D>
            {
                fn local_try_map_from(
                    src: ::std::vec::Vec<S>,
                ) -> ::core::result::Result<Self, $crate::MappingError> {
                    src.into_iter().map(D::local_try_map_from).collect()
                }
            }

            /// Crate-local twin of [`magic_map::MapFrom`] — the infallible
            /// funnel. Like its fallible sibling it is a CLOSED WORLD: only
            /// infallible fn-form mappings and leaves that genuinely cannot
            /// fail are delegated in, which is what lets an `infallible`
            /// fn-form mapping nest another foreign→foreign mapping without
            /// giving up the no-`?` check.
            pub trait LocalMapFrom<S>: Sized {
                fn local_map_from(src: S) -> Self;
            }

            impl<S, D: LocalMapFrom<S>> LocalMapFrom<::core::option::Option<S>>
                for ::core::option::Option<D>
            {
                fn local_map_from(src: ::core::option::Option<S>) -> Self {
                    src.map(D::local_map_from)
                }
            }

            impl<S, D: LocalMapFrom<S>> LocalMapFrom<::std::vec::Vec<S>> for ::std::vec::Vec<D> {
                fn local_map_from(src: ::std::vec::Vec<S>) -> Self {
                    src.into_iter().map(D::local_map_from).collect()
                }
            }

            $crate::__magic_map_scope_leaves!();
            $($from::__magic_map_leaves!();)*
            $crate::__magic_map_scope_extra_leaves!( $($leaf)* );
            $crate::__magic_map_scope_generic_leaves!( $($generic)* );

            // `..Default::default()` field machinery over the local trait.
            // Three autoref tiers told apart by receiver shape, mirroring
            // `magic_map::MapPair`'s.
            pub trait LocalFieldOpt<D> {
                fn local_map_field_or(self) -> ::core::result::Result<D, $crate::MappingError>;
            }
            impl<S, D: LocalTryMapFrom<S>> LocalFieldOpt<D>
                for &mut &mut &mut $crate::MapPair<::core::option::Option<S>, D>
            {
                fn local_map_field_or(self) -> ::core::result::Result<D, $crate::MappingError> {
                    match self.0.take().expect("magic_map field consumed twice") {
                        ::core::option::Option::Some(s) => D::local_try_map_from(s),
                        ::core::option::Option::None => ::core::result::Result::Ok(
                            self.1.take().expect("magic_map fallback consumed twice"),
                        ),
                    }
                }
            }

            pub trait LocalFieldVal<D> {
                fn local_map_field_or(self) -> ::core::result::Result<D, $crate::MappingError>;
            }
            impl<S, D: LocalTryMapFrom<S>> LocalFieldVal<D> for &mut &mut $crate::MapPair<S, D> {
                fn local_map_field_or(self) -> ::core::result::Result<D, $crate::MappingError> {
                    D::local_try_map_from(self.0.take().expect("magic_map field consumed twice"))
                }
            }

            pub trait LocalFieldWrap<D> {
                fn local_map_field_or(self) -> ::core::result::Result<D, $crate::MappingError>;
            }
            impl<S, U: LocalTryMapFrom<S>> LocalFieldWrap<::core::option::Option<U>>
                for &mut $crate::MapPair<S, ::core::option::Option<U>>
            {
                fn local_map_field_or(
                    self,
                ) -> ::core::result::Result<::core::option::Option<U>, $crate::MappingError>
                {
                    let src = self.0.take().expect("magic_map field consumed twice");
                    ::core::result::Result::Ok(::core::option::Option::Some(
                        U::local_try_map_from(src)?,
                    ))
                }
            }
        }
    };
}

/// Delegates one `TryMapFrom` pair into the local fallible trait. Used by
/// `magic_map_scope!` for both the built-in leaves and the `leaves: [...]`
/// list; the body is a plain call, so a missing `TryMapFrom` impl fails here and
/// names the pair.
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_delegate {
    ($src:ty => $dest:ty) => {
        impl LocalTryMapFrom<$src> for $dest {
            fn local_try_map_from(
                src: $src,
            ) -> ::core::result::Result<Self, $crate::MappingError> {
                <$dest as $crate::TryMapFrom<$src>>::try_map_from(src)
            }
        }
    };
}

/// Delegates one `MapFrom` pair into the local infallible trait — and into the
/// fallible one, so an infallible leaf serves both funnels from a single
/// declaration. A pair that is not actually infallible fails here on the
/// missing `MapFrom` impl, naming both types.
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_delegate_infallible {
    ($src:ty => $dest:ty) => {
        impl LocalMapFrom<$src> for $dest {
            fn local_map_from(src: $src) -> Self {
                <$dest as $crate::MapFrom<$src>>::map_from(src)
            }
        }
        impl LocalTryMapFrom<$src> for $dest {
            fn local_try_map_from(
                src: $src,
            ) -> ::core::result::Result<Self, $crate::MappingError> {
                ::core::result::Result::Ok(<$dest as $crate::MapFrom<$src>>::map_from(src))
            }
        }
    };
}

/// The `leaves: [...]` entries. A bare type is its identity (infallible by
/// definition); `infallible Src => Dest` requires a `MapFrom` route and feeds
/// both funnels; a plain `Src => Dest` delegates fallibly.
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_extra_leaves {
    () => {};
    (infallible $src:ty => $dest:ty, $($rest:tt)*) => {
        $crate::__magic_map_scope_delegate_infallible!($src => $dest);
        $crate::__magic_map_scope_extra_leaves!($($rest)*);
    };
    (infallible $src:ty => $dest:ty $(,)?) => {
        $crate::__magic_map_scope_delegate_infallible!($src => $dest);
    };
    ($src:ty => $dest:ty, $($rest:tt)*) => {
        $crate::__magic_map_scope_delegate!($src => $dest);
        $crate::__magic_map_scope_extra_leaves!($($rest)*);
    };
    ($src:ty => $dest:ty $(,)?) => {
        $crate::__magic_map_scope_delegate!($src => $dest);
    };
    ($t:ty, $($rest:tt)*) => {
        $crate::__magic_map_scope_delegate_infallible!($t => $t);
        $crate::__magic_map_scope_extra_leaves!($($rest)*);
    };
    ($t:ty $(,)?) => {
        $crate::__magic_map_scope_delegate_infallible!($t => $t);
    };
}

/// `generic_leaves { .. }` entries — one delegating impl per `;`-separated
/// declaration, generics and bounds passed through verbatim.
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_generic_leaves {
    () => {};
    (
        < $($gen:tt),* $(,)? > $src:ty => $dest:ty where $($bound:tt)*
    ) => {
        $crate::__magic_map_scope_generic_one!( < $($gen),* > $src => $dest where $($bound)* );
    };
    (
        < $($gen:tt),* $(,)? > $src:ty => $dest:ty ; $($rest:tt)*
    ) => {
        impl< $($gen),* > LocalTryMapFrom<$src> for $dest {
            fn local_try_map_from(
                src: $src,
            ) -> ::core::result::Result<Self, $crate::MappingError> {
                <$dest as $crate::TryMapFrom<$src>>::try_map_from(src)
            }
        }
        $crate::__magic_map_scope_generic_leaves!( $($rest)* );
    };
}

/// Terminal arm: splits a `where` clause off at its trailing `;`.
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_generic_one {
    (
        < $($gen:tt),* > $src:ty => $dest:ty where $($bound:tt)*
    ) => {
        $crate::__magic_map_scope_generic_split!(
            [ $($gen),* ] [ $src ] [ $dest ] [] $($bound)*
        );
    };
}

/// Walks the `where` clause token by token until the `;` that ends this
/// declaration, then emits the impl and recurses on what follows.
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_generic_split {
    ( [ $($gen:tt),* ] [ $src:ty ] [ $dest:ty ] [ $($bound:tt)* ] ; $($rest:tt)* ) => {
        impl< $($gen),* > LocalTryMapFrom<$src> for $dest where $($bound)* {
            fn local_try_map_from(
                src: $src,
            ) -> ::core::result::Result<Self, $crate::MappingError> {
                <$dest as $crate::TryMapFrom<$src>>::try_map_from(src)
            }
        }
        $crate::__magic_map_scope_generic_leaves!( $($rest)* );
    };
    ( [ $($gen:tt),* ] [ $src:ty ] [ $dest:ty ] [ $($bound:tt)* ] $next:tt $($rest:tt)* ) => {
        $crate::__magic_map_scope_generic_split!(
            [ $($gen),* ] [ $src ] [ $dest ] [ $($bound)* $next ] $($rest)*
        );
    };
}

/// The built-in leaf set, delegated into a scope's local trait. Mirrors the
/// `leaf_identity!` / `leaf_widen!` / `*_leaves` impls above one for one —
/// `scope_covers_every_builtin_leaf` in the test suite fails if the two drift.
///
/// The feature-gated groups are separate macros rather than `#[cfg]` arms in
/// the expansion: an emitted `#[cfg(feature = "uuid")]` is evaluated against
/// the *calling* crate's features, which has no `uuid` feature and would drop
/// every Uuid leaf on the floor. Defining each group's macro under the cfg
/// here evaluates it against this crate's features, where it means something.
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_leaves {
    () => {
        $crate::__magic_map_scope_extra_leaves!(
            bool, char, i8, i16, i32, i64, i128, isize,
            u8, u16, u32, u64, u128, usize, f32, f64,
            ::std::string::String,
        );
        // Lossless widenings — infallible by definition.
        $crate::__magic_map_scope_extra_leaves!(
            infallible u8 => u16, infallible u8 => u32, infallible u8 => u64,
            infallible u8 => i16, infallible u8 => i32, infallible u8 => i64,
            infallible u16 => u32, infallible u16 => u64,
            infallible u16 => i32, infallible u16 => i64,
            infallible u32 => u64, infallible u32 => i64,
            infallible i8 => i16, infallible i8 => i32, infallible i8 => i64,
            infallible i16 => i32, infallible i16 => i64,
            infallible i32 => i64,
            infallible f32 => f64,
        );
        $crate::__magic_map_scope_uuid_leaves!();
        $crate::__magic_map_scope_decimal_leaves!();
        $crate::__magic_map_scope_chrono_leaves!();
        $crate::__magic_map_scope_json_leaves!();
    };
}

#[cfg(feature = "uuid")]
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_uuid_leaves {
    () => {
        $crate::__magic_map_scope_extra_leaves!(
            $crate::__rx::uuid::Uuid,
            ::std::string::String => $crate::__rx::uuid::Uuid,
            infallible $crate::__rx::uuid::Uuid => ::std::string::String,
        );
    };
}
#[cfg(not(feature = "uuid"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_uuid_leaves {
    () => {};
}

#[cfg(feature = "decimal")]
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_decimal_leaves {
    () => {
        $crate::__magic_map_scope_extra_leaves!(
            $crate::__rx::rust_decimal::Decimal,
            $crate::__rx::rust_decimal::Decimal => f64,
            f64 => $crate::__rx::rust_decimal::Decimal,
            ::std::string::String => $crate::__rx::rust_decimal::Decimal,
            infallible $crate::__rx::rust_decimal::Decimal => ::std::string::String,
        );
    };
}
#[cfg(not(feature = "decimal"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_decimal_leaves {
    () => {};
}

#[cfg(feature = "chrono")]
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_chrono_leaves {
    () => {
        $crate::__magic_map_scope_extra_leaves!(
            $crate::__rx::chrono::DateTime<$crate::__rx::chrono::Utc>,
            $crate::__rx::chrono::NaiveDate,
            $crate::__rx::chrono::NaiveDateTime,
            $crate::__rx::chrono::NaiveTime,
            ::std::string::String => $crate::__rx::chrono::DateTime<$crate::__rx::chrono::Utc>,
            infallible $crate::__rx::chrono::DateTime<$crate::__rx::chrono::Utc> => ::std::string::String,
            ::std::string::String => $crate::__rx::chrono::NaiveDate,
            infallible $crate::__rx::chrono::NaiveDate => ::std::string::String,
        );
    };
}
#[cfg(not(feature = "chrono"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_chrono_leaves {
    () => {};
}

#[cfg(feature = "json")]
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_json_leaves {
    () => {
        $crate::__magic_map_scope_extra_leaves!($crate::__rx::serde_json::Value);
    };
}
#[cfg(not(feature = "json"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_scope_json_leaves {
    () => {};
}

/// One `LocalMapFrom` impl delegating to an existing `TryMapFrom` pair. Emitted by
/// a crate's replayed leaf list and by `magic_map_scope!`'s own `leaves`; the
/// bare `LocalMapFrom` binds to whichever scope module it lands in.
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_leaf_impl {
    ($src:ty => $dest:ty) => {
        impl LocalMapFrom<$src> for $dest {
            fn local_map_from(src: $src) -> ::core::result::Result<Self, $crate::MappingError> {
                <$dest as $crate::TryMapFrom<$src>>::try_map_from(src)
            }
        }
    };
}
