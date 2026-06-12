//! Procedural macros for the [`magic_map`](https://crates.io/crates/magic_map)
//! crate. Depend on `magic_map` instead — it re-exports everything here and
//! provides the trait machinery the generated code needs.

use proc_macro::TokenStream;
use syn::parse_macro_input;

mod magic_map;

/// `#[derive(MagicMap)]` — content-free metadata derive that makes a type
/// usable as a `magic_map!` destination (or open tuple-element source).
///
/// It re-publishes the type's own field (or variant) names as a hidden macro
/// next to the type — nothing else. It never references another layer, so it
/// is safe on db models, dtos, and (via prost-build's `type_attribute`)
/// generated proto types alike. The actual mappings are declared with
/// `magic_map!` wherever your conversions live.
///
/// Named-field structs and unit-variant enums get a schema; other shapes are
/// a silent no-op (so the derive can be applied blanket-style, e.g. via
/// prost-build `type_attribute(".", ..)`). Two same-named derived types in
/// one crate collide on the hidden `#[macro_export]` — disambiguate the
/// export of one with `#[magic_map(export = "UniqueName")]` (the name
/// `magic_map!` resolves is module-scoped and unaffected).
#[proc_macro_derive(MagicMap, attributes(magic_map))]
pub fn derive_magic_map(input: TokenStream) -> TokenStream {
    magic_map::derive(parse_macro_input!(input as syn::DeriveInput))
        .unwrap_or_else(|e| e.to_compile_error().into())
}

/// `magic_map!` — declare a struct/enum mapping at the call site.
///
/// ```ignore
/// // impl form → `impl magic_map::MapFrom<Src> for Dest`
/// // (orphan rule: Src or Dest must be local to the calling crate)
/// magic_map!(db::LicenseStatus => super::dtos::LicenseStatusDto);
/// magic_map!(db::License => super::dtos::LicenseResponse {
///     devices_used: 0,                                   // absent from src
///     license_type: parse_license_type(&src.license_type), // custom expr
/// });
///
/// // tuple source → `impl MapFrom<(License, i64)> for LicenseResponse`;
/// // call sites do `(license, count).map_into()?`.
/// magic_map!((db::License, i64) => super::dtos::LicenseResponse {
///     devices_used: src.1 as i32,
///     license_type: parse_license_type(&src.0.license_type),
/// });
///
/// // fn form → plain `pub fn name(src: Src) -> Result<Dest, MappingError>`
/// // for foreign→foreign mappings (db→proto in a service crate) where the
/// // orphan rule forbids any impl.
/// magic_map!(pub fn equipment_to_proto: db::Equipment => proto::Module);
/// ```
///
/// Every destination field without an override is auto-filled from the
/// same-named source field through the `MapFrom` leaf funnel (identities,
/// String↔Uuid, Decimal↔f64, `Option`/`Vec` wrappers, mapped enums/structs).
/// Override expressions may use `src` (the whole source value). Enum mappings
/// are variant-by-name and take `SrcVariant => DestVariant` rename pairs
/// instead of field overrides.
///
/// A trailing `..Default::default()` makes the mapping default-tolerant
/// (the destination must implement `Default` — put business defaults on the
/// model; a single-path source must derive `MagicMap`):
///
/// - dest fields absent from every source come from `Default::default()`;
/// - `Option<S>` source fields landing in non-`Option` dest fields unwrap
///   through the funnel and fall back to the default instance's field value
///   on `None` — `unwrap_or(business_default)` lines disappear entirely.
///
/// ```ignore
/// magic_map!(dtos::CreateLicenseRequest => db::CreateLicense { ..Default::default() });
/// ```
///
/// Keep it for sparse update/create models — it trades the missing-field
/// compile error for silent defaulting.
///
/// Tuple sources: plain non-generic struct elements are OPEN — they must
/// derive `MagicMap` and their fields join the auto-match. Leaves and generic
/// types (`i64`, `Option<String>`, `DateTime<Utc>`) are OPAQUE — reachable
/// only as `src.N` in overrides. A destination field found in exactly one
/// open element auto-maps from it; found in several (or none) is a compile
/// error asking for an explicit override.
///
/// `Dest` must derive `MagicMap`, which plants a schema macro right next to
/// the type. Spell `Dest` as any qualified path that reaches the type
/// (`super::dtos::LicenseResponse`, `db::License`); a bare ident only works
/// if the schema macro is in scope too (glob import).
#[proc_macro]
pub fn magic_map(input: TokenStream) -> TokenStream {
    magic_map::front(parse_macro_input!(input as magic_map::MagicMapInput))
}

/// Internal back end of `magic_map!` — the generated schema macros call this.
/// Not for direct use.
#[doc(hidden)]
#[proc_macro]
pub fn __magic_map_expand(input: TokenStream) -> TokenStream {
    let raw: proc_macro2::TokenStream = input.clone().into();
    magic_map::expand(raw, parse_macro_input!(input as magic_map::ExpandInput))
        .unwrap_or_else(|e| e.to_compile_error().into())
}
