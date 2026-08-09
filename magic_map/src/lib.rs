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
//! A crate using the fn form calls [`magic_map_scope!`] once in its crate
//! root. That plants the crate-local funnel the generated functions resolve
//! nested fields through, so foreign→foreign mappings compose with each other
//! the same way impl-form ones do — see the macro's docs for the whole story,
//! including why custom leaves have to be named there.
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
//! [`map_display!`], [`map_parse!`], or a plain `MapFrom` impl in the crate
//! that owns the type.

use std::error::Error;
use std::fmt;

pub use magic_map_macros::{magic_map, MagicMap};

#[doc(hidden)]
pub use magic_map_macros::__magic_map_expand;

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

// ── Crate-local funnel for foreign→foreign mappings ─────────────────────────
//
// The fn form exists because `impl MapFrom<Src> for Dest` is only legal in a
// crate owning one of the two types. But a mapping's *fields* funnelled through
// `MapFrom` too, so a nested field whose own mapping was also foreign→foreign
// had nothing to resolve against: `Vec<Address>` → `Vec<AddressResponse>` needs
// `AddressResponse: MapFrom<Address>`, and that impl cannot exist anywhere.
//
// `magic_map_scope!()` plants a trait in the *calling* crate. The orphan rule
// is satisfied by a local trait just as well as by a local type, so
// `impl LocalMapFrom<Address> for AddressResponse` is legal there even though
// both types are foreign — and the fn form emits one for every mapping it
// declares.
//
// Leaves need no declaration. Field resolution probes two tiers by autoref:
// the fn form emits CONCRETE tier-1 impls, one per declared pair, so a leaf
// pair has no tier-1 candidate at all and probing derefs to the tier-2 blanket
// over `MapFrom`. That is the whole trick — a *blanket* tier 1 would match
// every pair structurally and then hard-error on its unsatisfied bound instead
// of falling through, which is why the tiers cannot be written the obvious way.
// One consequence worth stating: there is exactly one way to register a
// conversion, which is to declare it. Nothing else needs configuring, ever.

/// Declares the crate-local mapping funnel that the fn form of `magic_map!`
/// resolves nested fields through. Call it **once, in the crate root**
/// (`lib.rs` / `main.rs`) of any crate that declares a fn-form mapping:
///
/// ```ignore
/// // src/lib.rs
/// magic_map::magic_map_scope!();
/// ```
///
/// It takes no configuration. Leaves — the built-in ones, your
/// [`map_identity!`] / [`map_display!`] / [`map_parse!`] declarations, and any
/// `MapFrom` impl you wrote by hand — are found without being named.
///
/// Needed only for the fn form. A crate whose mappings are all impl form
/// (`magic_map!(Src => Dest)`, where one side is local) never calls it — those
/// funnel through `MapFrom` as they always have.
///
/// Missing it reads as ``could not find `__magic_map_scope` in the crate root``
/// at the first fn-form mapping.
#[macro_export]
macro_rules! magic_map_scope {
    () => {
        #[doc(hidden)]
        pub mod __magic_map_scope {
            //! Generated by `magic_map::magic_map_scope!`. The fn form of
            //! `magic_map!` resolves its fields through the traits here.

            #[allow(unused_imports)]
            use super::*;

            /// Crate-local twin of [`magic_map::MapFrom`], implemented by the
            /// fn form for pairs the orphan rule keeps off `MapFrom`.
            pub trait LocalMapFrom<S>: Sized {
                fn local_map_from(src: S) -> ::core::result::Result<Self, $crate::MappingError>;
            }

            impl<S, D: LocalMapFrom<S>> LocalMapFrom<::core::option::Option<S>>
                for ::core::option::Option<D>
            {
                fn local_map_from(
                    src: ::core::option::Option<S>,
                ) -> ::core::result::Result<Self, $crate::MappingError> {
                    match src {
                        ::core::option::Option::Some(s) => ::core::result::Result::Ok(
                            ::core::option::Option::Some(D::local_map_from(s)?),
                        ),
                        ::core::option::Option::None => {
                            ::core::result::Result::Ok(::core::option::Option::None)
                        }
                    }
                }
            }

            impl<S, D: LocalMapFrom<S>> LocalMapFrom<::std::vec::Vec<S>> for ::std::vec::Vec<D> {
                fn local_map_from(
                    src: ::std::vec::Vec<S>,
                ) -> ::core::result::Result<Self, $crate::MappingError> {
                    src.into_iter().map(D::local_map_from).collect()
                }
            }

            // ── Plain field resolution: two tiers ───────────────────────────
            //
            // Tier 1 is populated only by the concrete impls the fn form emits
            // per declared pair (see `__magic_map_declare_local!`). A leaf has
            // no candidate here, so probing derefs to tier 2.

            pub trait ProbeLocal<D> {
                fn magic_probe(self) -> ::core::result::Result<D, $crate::MappingError>;
            }

            /// Tier 2 — every conversion that already has a `MapFrom` impl:
            /// built-in leaves, your own leaf declarations, hand-written impls,
            /// and the `Option`/`Vec` blankets over them.
            pub trait ProbeGlobal<D> {
                fn magic_probe(self) -> ::core::result::Result<D, $crate::MappingError>;
            }
            impl<S, D: $crate::MapFrom<S>> ProbeGlobal<D> for &mut $crate::MapProbe<S, D> {
                fn magic_probe(self) -> ::core::result::Result<D, $crate::MappingError> {
                    D::map_from(self.0.take().expect("magic_map field consumed twice"))
                }
            }

            // ── `..Default::default()` field resolution: six tiers ──────────
            //
            // The three shapes of the `MapFrom` version, doubled: local first,
            // then global. All six share a method name and are told apart by
            // autoref depth, so the local ones win where they have a candidate
            // and leaves fall straight through to the global three.

            pub trait LocalFieldOpt<D> {
                fn magic_field(self) -> ::core::result::Result<D, $crate::MappingError>;
            }
            pub trait LocalFieldVal<D> {
                fn magic_field(self) -> ::core::result::Result<D, $crate::MappingError>;
            }
            pub trait LocalFieldWrap<D> {
                fn magic_field(self) -> ::core::result::Result<D, $crate::MappingError>;
            }

            pub trait GlobalFieldOpt<D> {
                fn magic_field(self) -> ::core::result::Result<D, $crate::MappingError>;
            }
            impl<S, D: $crate::MapFrom<S>> GlobalFieldOpt<D>
                for &mut &mut &mut $crate::MapPair<::core::option::Option<S>, D>
            {
                fn magic_field(self) -> ::core::result::Result<D, $crate::MappingError> {
                    match self.0.take().expect("magic_map field consumed twice") {
                        ::core::option::Option::Some(s) => D::map_from(s),
                        ::core::option::Option::None => ::core::result::Result::Ok(
                            self.1.take().expect("magic_map fallback consumed twice"),
                        ),
                    }
                }
            }

            pub trait GlobalFieldVal<D> {
                fn magic_field(self) -> ::core::result::Result<D, $crate::MappingError>;
            }
            impl<S, D: $crate::MapFrom<S>> GlobalFieldVal<D> for &mut &mut $crate::MapPair<S, D> {
                fn magic_field(self) -> ::core::result::Result<D, $crate::MappingError> {
                    D::map_from(self.0.take().expect("magic_map field consumed twice"))
                }
            }

            pub trait GlobalFieldWrap<D> {
                fn magic_field(self) -> ::core::result::Result<D, $crate::MappingError>;
            }
            impl<S, U: $crate::MapFrom<S>> GlobalFieldWrap<::core::option::Option<U>>
                for &mut $crate::MapPair<S, ::core::option::Option<U>>
            {
                fn magic_field(
                    self,
                ) -> ::core::result::Result<::core::option::Option<U>, $crate::MappingError>
                {
                    let src = self.0.take().expect("magic_map field consumed twice");
                    ::core::result::Result::Ok(::core::option::Option::Some(U::map_from(src)?))
                }
            }
        }
    };
}

/// Registers one declared pair in the crate-local funnel. Emitted by the fn
/// form of `magic_map!` — the concrete tier-1 impls that let a leaf fall
/// through to `MapFrom` while a declared pair does not.
#[doc(hidden)]
#[macro_export]
macro_rules! __magic_map_declare_local {
    // `$scope` is the caller's `crate::__magic_map_scope`, passed in by the
    // proc macro rather than spelled `crate::` here — inside a macro_rules
    // definition that would read as this crate, which is not what is meant.
    ($scope:path, $fn_name:ident, $src:ty, $dest:ty) => {
        const _: () = {
            use $scope::*;

            impl LocalMapFrom<$src> for $dest {
                fn local_map_from(src: $src) -> ::core::result::Result<Self, $crate::MappingError> {
                    $fn_name(src)
                }
            }

            // Plain resolution, and the two wrappers a DTO field actually uses.
            impl ProbeLocal<$dest> for &mut &mut $crate::MapProbe<$src, $dest> {
                fn magic_probe(self) -> ::core::result::Result<$dest, $crate::MappingError> {
                    $fn_name(self.0.take().expect("magic_map field consumed twice"))
                }
            }
            impl ProbeLocal<::std::vec::Vec<$dest>>
                for &mut &mut $crate::MapProbe<::std::vec::Vec<$src>, ::std::vec::Vec<$dest>>
            {
                fn magic_probe(
                    self,
                ) -> ::core::result::Result<::std::vec::Vec<$dest>, $crate::MappingError> {
                    <::std::vec::Vec<$dest> as LocalMapFrom<::std::vec::Vec<$src>>>::local_map_from(
                        self.0.take().expect("magic_map field consumed twice"),
                    )
                }
            }
            impl ProbeLocal<::core::option::Option<$dest>>
                for &mut &mut $crate::MapProbe<
                    ::core::option::Option<$src>,
                    ::core::option::Option<$dest>,
                >
            {
                fn magic_probe(
                    self,
                ) -> ::core::result::Result<::core::option::Option<$dest>, $crate::MappingError>
                {
                    <::core::option::Option<$dest> as LocalMapFrom<
                                        ::core::option::Option<$src>,
                                    >>::local_map_from(
                                        self.0.take().expect("magic_map field consumed twice")
                                    )
                }
            }

            // `..Default::default()` shapes.
            impl LocalFieldOpt<$dest>
                for &mut &mut &mut &mut &mut &mut $crate::MapPair<
                    ::core::option::Option<$src>,
                    $dest,
                >
            {
                fn magic_field(self) -> ::core::result::Result<$dest, $crate::MappingError> {
                    match self.0.take().expect("magic_map field consumed twice") {
                        ::core::option::Option::Some(s) => $fn_name(s),
                        ::core::option::Option::None => ::core::result::Result::Ok(
                            self.1.take().expect("magic_map fallback consumed twice"),
                        ),
                    }
                }
            }
            impl LocalFieldVal<$dest> for &mut &mut &mut &mut &mut $crate::MapPair<$src, $dest> {
                fn magic_field(self) -> ::core::result::Result<$dest, $crate::MappingError> {
                    $fn_name(self.0.take().expect("magic_map field consumed twice"))
                }
            }
            impl LocalFieldWrap<::core::option::Option<$dest>>
                for &mut &mut &mut &mut $crate::MapPair<$src, ::core::option::Option<$dest>>
            {
                fn magic_field(
                    self,
                ) -> ::core::result::Result<::core::option::Option<$dest>, $crate::MappingError>
                {
                    let src = self.0.take().expect("magic_map field consumed twice");
                    ::core::result::Result::Ok(::core::option::Option::Some($fn_name(src)?))
                }
            }
        };
    };
}

#[doc(hidden)]
pub struct MapProbe<S, D>(pub Option<S>, pub core::marker::PhantomData<D>);

impl<S, D> MapProbe<S, D> {
    #[doc(hidden)]
    pub fn new(src: S) -> Self {
        MapProbe(Some(src), core::marker::PhantomData)
    }
}
