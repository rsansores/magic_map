//! A leaf-owning crate, as a consumer of `magic_map` would write one.

// One block, in the crate root. Emits the `TryMapFrom` impls and publishes the
// pair list; nothing downstream restates any of it.
magic_map::magic_map_leaves! {
    identity: [crate::enums::Species],
    display: [crate::enums::Species],
    parse: [crate::enums::Species],
    custom: [
        crate::wire::Fahrenheit => String,
        infallible crate::wire::Celsius => String,
    ],
}

pub mod enums {
    #[derive(
        Clone, Copy, Debug, PartialEq, strum::Display, strum::EnumString, magic_map::MagicMap,
    )]
    pub enum Species {
        Cat,
        Lion,
    }
}

pub mod wire {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Fahrenheit(pub i32);

    // Hand-written, so no macro can see it — the pair is registered in the
    // block above instead.
    impl magic_map::TryMapFrom<Fahrenheit> for String {
        fn try_map_from(src: Fahrenheit) -> Result<Self, magic_map::MappingError> {
            Ok(format!("{}F", src.0))
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Celsius(pub i32);

    // An infallible custom leaf carries one `MapFrom` impl; the `infallible`
    // entry in the block above backs both local funnels with it.
    impl magic_map::MapFrom<Celsius> for String {
        fn map_from(src: Celsius) -> Self {
            format!("{}C", src.0)
        }
    }
}

/// A model carrying both leaves, for a downstream mapper to convert.
#[derive(magic_map::MagicMap)]
pub struct Reading {
    pub species: enums::Species,
    pub species_label: enums::Species,
    pub temp: wire::Fahrenheit,
}

// ── a sealed type, to be built from another crate ────────────────────────────
/// Sealed: no other crate can write a struct expression for this, so the only
/// way in is `magic_map!` (which uses the hidden constructor) or the deliberate,
/// greppable call.
#[magic_map::mapped(sealed)]
#[derive(Debug, PartialEq, Default)]
pub struct SealedDto {
    pub id: String,
    pub count: u64,
}

/// The same shape, unsealed, as the control.
#[magic_map::mapped]
#[derive(Debug, PartialEq, Default)]
pub struct OpenDto {
    pub id: String,
    pub count: u64,
}
