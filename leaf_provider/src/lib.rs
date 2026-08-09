//! A leaf-owning crate, as a consumer of `magic_map` would write one.

// One block, in the crate root. Emits the `MapFrom` impls and publishes the
// pair list; nothing downstream restates any of it.
magic_map::magic_map_leaves! {
    identity: [crate::enums::Species],
    display: [crate::enums::Species],
    parse: [crate::enums::Species],
    custom: [crate::wire::Fahrenheit => String],
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
    impl magic_map::MapFrom<Fahrenheit> for String {
        fn map_from(src: Fahrenheit) -> Result<Self, magic_map::MappingError> {
            Ok(format!("{}F", src.0))
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
