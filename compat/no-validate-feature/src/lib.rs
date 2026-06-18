//! Regression guard for the `validate` feature gating.
//!
//! A struct can derive `validator::Validate` and carry `#[validate(...)]`
//! attributes for the user's own purposes while mapping with `magic_map`
//! *without* the `validate` feature. In that configuration magic_map must not
//! emit a `.validate()` call (nor reference `MappingError::Validation`, which
//! does not exist here) — it just maps the fields. If this crate compiles, the
//! gating holds.

use magic_map::{magic_map, MagicMap, MapInto};
use validator::Validate;

#[derive(MagicMap)]
pub struct RawInput {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Validate, MagicMap)]
pub struct ValidatedDto {
    #[validate(length(min = 1, max = 50))]
    pub name: String,
    #[validate(email)]
    pub email: String,
}

magic_map!(RawInput => ValidatedDto);

/// With the feature off, mapping does NOT validate: even constraint-violating
/// input maps to `Ok`.
pub fn maps_without_validating() -> ValidatedDto {
    RawInput {
        name: String::new(),
        email: "not-an-email".into(),
    }
    .map_into()
    .unwrap()
}
