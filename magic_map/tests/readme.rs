//! Mirrors the README examples — what the README claims is what CI compiles
//! and runs. Deviations from the verbatim snippets: extra
//! `#[magic_map(export = "...")]` attributes where this single test crate
//! would collide on same-named types (each README snippet is its own crate
//! context), and section modules wrapping each example.

// Example types exist to be mapped, not exhaustively read back.
#![allow(dead_code)]

use magic_map::{MapInto, MappingError};

// ── Quick start ──────────────────────────────────────────────────────────────

mod quickstart {
    pub mod db {
        #[derive(magic_map::MagicMap)]
        pub struct Cat {
            pub id: uuid::Uuid,
            pub name: String,
            pub age: i32,
            pub born: chrono::DateTime<chrono::Utc>,
        }
    }

    pub mod api {
        #[derive(Debug, magic_map::MagicMap)]
        pub struct CatDto {
            pub id: String,   // Uuid → String automaps (uuid leaf)
            pub name: String, // identity
            pub age: i64,     // i32 → i64 widens losslessly, automaps
            pub born: String, // DateTime<Utc> → rfc3339 String (chrono leaf)
        }
    }

    mod mappers {
        use magic_map::magic_map;
        // The only place that knows both layers.
        magic_map!(super::db::Cat => super::api::CatDto);
    }
}

#[test]
fn quick_start() -> Result<(), MappingError> {
    use quickstart::{api, db};

    let dto: api::CatDto = db::Cat {
        id: uuid::Uuid::nil(),
        name: "Misifu".into(),
        age: 3,
        born: "2024-01-15T10:30:00Z".parse().unwrap(),
    }
    .map_into()?;

    assert_eq!(dto.id, "00000000-0000-0000-0000-000000000000");
    assert_eq!(dto.name, "Misifu");
    assert_eq!(dto.age, 3_i64);
    assert_eq!(dto.born, "2024-01-15T10:30:00+00:00");
    Ok(())
}

// ── impl form ────────────────────────────────────────────────────────────────

mod impl_form {
    pub mod db {
        #[derive(magic_map::MagicMap)]
        pub struct Dog {
            pub name: String,
            pub weight_kg: f32,
        }
    }

    pub mod api {
        #[derive(Debug, magic_map::MagicMap)]
        pub struct DogDto {
            pub name: String,   // automaps (identity)
            pub weight_kg: f64, // automaps (f32 → f64 widens)
            pub big: bool,      // absent from the source → must be overridden
        }
    }

    use magic_map::magic_map;
    magic_map!(db::Dog => api::DogDto {
        big: src.weight_kg > 30.0,
    });
}

#[test]
fn impl_form() -> Result<(), MappingError> {
    use impl_form::{api, db};

    let dto: api::DogDto = db::Dog {
        name: "Rex".into(),
        weight_kg: 38.5,
    }
    .map_into()?;
    assert_eq!(dto.name, "Rex");
    assert!(dto.big);
    Ok(())
}

// ── fn form ──────────────────────────────────────────────────────────────────

mod fn_form {
    pub mod wire {
        #[derive(magic_map::MagicMap)]
        #[magic_map(export = "WireLion")]
        pub struct Lion {
            pub id: String,
            pub name: String,
        }
    }

    pub mod db {
        #[derive(Debug, magic_map::MagicMap)]
        pub struct Lion {
            pub id: uuid::Uuid, // String → Uuid parses strictly
            pub name: String,
        }
    }

    use magic_map::magic_map;
    magic_map!(pub fn lion_to_db: wire::Lion => db::Lion);
}

#[test]
fn fn_form() -> Result<(), MappingError> {
    use fn_form::{lion_to_db, wire};

    let row = lion_to_db(wire::Lion {
        id: "67e55044-10b1-426f-9247-bb680e5fe0c8".into(),
        name: "Simba".into(),
    })?;
    assert_eq!(row.name, "Simba");

    // Strict by default: garbage is an Err, not a Uuid::nil().
    let bad = lion_to_db(wire::Lion {
        id: "not-a-uuid".into(),
        name: "?".into(),
    });
    assert!(bad.is_err());
    Ok(())
}

// ── `let` preludes ───────────────────────────────────────────────────────────

mod preludes {
    pub mod geom {
        #[derive(magic_map::MagicMap)]
        pub struct Span {
            pub start: i64,
            pub end: i64,
        }

        #[derive(Debug, magic_map::MagicMap)]
        pub struct SpanStats {
            pub width: i64,
            pub midpoint: i64,
        }
    }

    use magic_map::magic_map;
    magic_map!(pub fn span_stats: geom::Span => geom::SpanStats {
        let width = src.end - src.start;
        width: width,
        midpoint: src.start + width / 2,
    });
}

#[test]
fn let_preludes() -> Result<(), MappingError> {
    use preludes::{geom, span_stats};

    let stats = span_stats(geom::Span { start: 10, end: 20 })?;
    assert_eq!(stats.width, 10);
    assert_eq!(stats.midpoint, 15);
    Ok(())
}

// ── Tuple sources ────────────────────────────────────────────────────────────

mod tuples {
    pub mod db {
        #[derive(magic_map::MagicMap)]
        #[magic_map(export = "AdoptionCat")]
        pub struct Cat {
            pub id: i64,
            pub name: String,
            pub age: i32,
        }

        #[derive(magic_map::MagicMap)]
        pub struct Owner {
            pub id: i64,      // collides with Cat::id
            pub name: String, // collides with Cat::name
        }
    }

    pub mod api {
        #[derive(Debug, magic_map::MagicMap)]
        pub struct AdoptionCard {
            pub id: i64,
            pub name: String,
            pub age: i32,
            pub note: Option<String>,
        }
    }

    use magic_map::magic_map;
    magic_map!((db::Cat, db::Owner, Option<String>) => api::AdoptionCard {
        id: src.1.id,     // `id` exists in BOTH Cat and Owner → explicit pick
        name: src.0.name, // same for `name`
        note: src.2,      // exists in neither struct → from the opaque element
    });
    // `age` automaps — exactly one open element (Cat) has it.
}

#[test]
fn tuple_sources() -> Result<(), MappingError> {
    use tuples::{api, db};

    let card: api::AdoptionCard = (
        db::Cat {
            id: 7,
            name: "Misifu".into(),
            age: 3,
        },
        db::Owner {
            id: 99,
            name: "Ricardo".into(),
        },
        Some("indoor only".to_string()),
    )
        .map_into()?;

    assert_eq!(card.id, 99); // picked from Owner
    assert_eq!(card.name, "Misifu"); // picked from Cat
    assert_eq!(card.age, 3); // automapped, unambiguous
    assert_eq!(card.note.as_deref(), Some("indoor only"));
    Ok(())
}

// ── Enum mappings ────────────────────────────────────────────────────────────

mod enums {
    pub mod wire {
        // prost-style: proto3 forces a zero variant.
        #[derive(magic_map::MagicMap)]
        #[magic_map(export = "WireSpecies")]
        pub enum Species {
            Unspecified,
            Cat,
            Dog,
            BigCat,
        }
    }

    pub mod db {
        #[derive(Debug, Default, PartialEq, magic_map::MagicMap)]
        pub enum Species {
            #[default]
            Cat,
            Dog,
            Lion,
        }
    }

    use magic_map::magic_map;
    magic_map!(pub fn species_to_db: wire::Species => db::Species {
        BigCat => Lion, // explicit rename; the rest pair by name
    });
}

#[test]
fn enum_mappings() -> Result<(), MappingError> {
    use enums::{db, species_to_db, wire};

    assert_eq!(species_to_db(wire::Species::BigCat)?, db::Species::Lion); // renamed
    assert_eq!(species_to_db(wire::Species::Dog)?, db::Species::Dog); // same name
    assert_eq!(species_to_db(wire::Species::Unspecified)?, db::Species::Cat); // zero → #[default]
    Ok(())
}

// ── `..Default::default()` optionality adaptor ───────────────────────────────

mod defaults_trailer {
    pub mod api {
        #[derive(magic_map::MagicMap)]
        pub struct CreateCatRequest {
            pub name: String,
            pub lives: Option<i32>, // optional on the wire
            pub note: Option<String>,
        }
    }

    pub mod db {
        #[derive(Debug, magic_map::MagicMap)]
        pub struct CreateCat {
            pub name: String,
            pub lives: i32, // required in the row
            pub note: Option<String>,
            pub status: String, // not on the wire at all
        }

        impl Default for CreateCat {
            fn default() -> Self {
                CreateCat {
                    name: String::new(),
                    lives: 9, // the business default lives HERE
                    note: None,
                    status: "new".into(),
                }
            }
        }
    }

    use magic_map::magic_map;
    magic_map!(api::CreateCatRequest => db::CreateCat { ..Default::default() });
}

#[test]
fn defaults_trailer() -> Result<(), MappingError> {
    use defaults_trailer::{api, db};

    let row: db::CreateCat = api::CreateCatRequest {
        name: "Misifu".into(),
        lives: None,
        note: None,
    }
    .map_into()?;

    assert_eq!(row.name, "Misifu"); // plain funnel
    assert_eq!(row.lives, 9); // None → business default from the model
    assert_eq!(row.note, None); // Option → Option: None stays None
    assert_eq!(row.status, "new"); // absent from the request → Default::default()

    let row2: db::CreateCat = api::CreateCatRequest {
        name: "Firulais".into(),
        lives: Some(7),
        note: Some("bites".into()),
    }
    .map_into()?;
    assert_eq!(row2.lives, 7); // Some → unwrapped through the funnel
    assert_eq!(row2.note.as_deref(), Some("bites"));
    Ok(())
}

// ── Wrap tier (plain source → Option dest) ───────────────────────────────────

mod wrap_tier {
    pub mod seen {
        #[derive(magic_map::MagicMap)]
        pub struct CatSeen {
            pub chip_id: String,
            pub weight_kg: f32,
        }

        #[derive(Debug, Default, magic_map::MagicMap)]
        pub struct CatPatch {
            pub chip_id: Option<uuid::Uuid>, // String funnels to Uuid, wraps in Some
            pub weight_kg: Option<f64>,      // f32 widens to f64, wraps in Some
        }
    }

    use magic_map::magic_map;
    magic_map!(seen::CatSeen => seen::CatPatch { ..Default::default() });
}

#[test]
fn wrap_tier() -> Result<(), MappingError> {
    use wrap_tier::seen;

    let patch: seen::CatPatch = seen::CatSeen {
        chip_id: "67e55044-10b1-426f-9247-bb680e5fe0c8".into(),
        weight_kg: 4.2,
    }
    .map_into()?;
    assert!(patch.chip_id.is_some());
    assert!(patch.weight_kg.is_some());

    // Still strict: a bad chip_id is an Err — never Some(Uuid::nil()).
    let bad: Result<seen::CatPatch, _> = seen::CatSeen {
        chip_id: "not-a-uuid".into(),
        weight_kg: 4.2,
    }
    .map_into();
    assert!(bad.is_err());
    Ok(())
}

// ── Your own leaves ──────────────────────────────────────────────────────────

mod leaves {
    #[derive(Debug, Clone, PartialEq, strum::Display, strum::EnumString, magic_map::MagicMap)]
    #[magic_map(export = "LeafSpecies")]
    pub enum Species {
        Cat,
        Dog,
        Lion,
    }

    magic_map::map_identity!(Species); // Species → Species (model→model moves)
    magic_map::map_display!(Species); // Species → String fields automap
    magic_map::map_parse!(Species); // String → Species fields automap, strictly

    // Or hand-write any pair:
    pub struct MyWireTimestamp {
        pub seconds: i64,
    }

    impl magic_map::MapFrom<MyWireTimestamp> for chrono::DateTime<chrono::Utc> {
        fn map_from(src: MyWireTimestamp) -> Result<Self, magic_map::MappingError> {
            chrono::DateTime::from_timestamp(src.seconds, 0).ok_or(
                magic_map::MappingError::OutOfRange {
                    field: "<timestamp>",
                },
            )
        }
    }
}

#[test]
fn custom_leaves() -> Result<(), MappingError> {
    use leaves::{MyWireTimestamp, Species};

    let s: String = Species::Lion.map_into()?;
    assert_eq!(s, "Lion");

    let back: Species = "Lion".to_string().map_into()?;
    assert_eq!(back, Species::Lion);

    let bad: Result<Species, _> = "Liger".to_string().map_into();
    assert!(bad.is_err()); // strict parse

    let ts: chrono::DateTime<chrono::Utc> = MyWireTimestamp { seconds: 0 }.map_into()?;
    assert_eq!(ts.to_rfc3339(), "1970-01-01T00:00:00+00:00");
    Ok(())
}
