//! The mapping machinery is tested ONCE, here — against synthetic types that
//! mirror the db/dtos/mappers layering. Real mappers built on `magic_map!`
//! don't carry their own unit tests: asserting a generated field-by-field copy
//! is restating the declaration.

// Integration tests are their own crate, so the scope lives here rather than
// in a lib.rs — same rule: once, at the crate root.
magic_map::magic_map_scope!();
use magic_map::magic_map;
use magic_map::{MapFrom, MapInto, MappingError};
use rust_decimal::Decimal;
use uuid::Uuid;

/// Source layer — only the content-free metadata derive, no dto knowledge.
mod db {
    use chrono::{DateTime, Utc};
    use magic_map::MagicMap;
    use rust_decimal::Decimal;
    use uuid::Uuid;

    #[derive(Clone, Debug, PartialEq, MagicMap)]
    pub enum Status {
        Trial,
        Active,
        Suspended,
    }

    #[derive(Clone, Debug, MagicMap)]
    pub struct License {
        pub id: Uuid,
        pub status: Status,
        pub max_devices: i32,
        pub valid_until: Option<DateTime<Utc>>,
        pub price_usd: Option<Decimal>,
        pub license_type: String,
        pub notes: Option<String>,
        pub tags: Vec<String>,
    }

    #[derive(Clone, Debug, MagicMap)]
    pub struct Owner {
        pub id: Uuid,
        pub name: String,
    }
}

/// Destination layer — no db imports, no mapping attributes.
mod dtos {
    use chrono::{DateTime, Utc};
    use magic_map::MagicMap;
    use uuid::Uuid;

    #[derive(Clone, Debug, PartialEq, MagicMap)]
    pub enum StatusDto {
        Trial,
        Active,
        Suspended,
    }

    #[derive(Clone, Debug, MagicMap)]
    pub struct LicenseResponse {
        pub id: Uuid,
        pub status: StatusDto,
        pub max_devices: i32,
        pub devices_used: i32,
        pub valid_until: Option<DateTime<Utc>>,
        pub price_usd: Option<f64>,
        pub license_type: bool,
        pub notes: Option<String>,
        pub tags: Vec<String>,
    }

    #[derive(Clone, Debug, MagicMap)]
    pub struct StatusDto2Holder {
        pub code: i32,
        pub reason: String,
    }

    #[derive(Clone, Debug, MagicMap)]
    pub struct LicenseCard {
        pub id: Uuid,
        pub name: String,
        pub max_devices: i32,
        pub comment: Option<String>,
    }
}

/// The only place that knows both layers.
mod mappers {
    use super::db;
    use magic_map::magic_map;

    // Enum impl form, both directions.
    magic_map!(db::Status => super::dtos::StatusDto);
    magic_map!(super::dtos::StatusDto => db::Status);

    // Struct impl form: 2 overrides (one absent from src, one custom expr
    // referencing `src`); everything else auto-funnels.
    magic_map!(db::License => super::dtos::LicenseResponse {
        devices_used: 0,
        license_type: src.license_type == "custom",
    });

    // fn form — no trait impl, so it also works foreign→foreign (db→proto in
    // a neutral service crate), where the orphan rule forbids any impl.
    magic_map!(pub fn status_to_dto: db::Status => super::dtos::StatusDto);

    // Tuple source. `devices_used` comes from the opaque i64 element; the
    // remaining fields auto-match against License's schema.
    magic_map!((db::License, i64) => super::dtos::LicenseResponse {
        devices_used: src.1 as i32,
        license_type: src.0.license_type == "custom",
    });

    // Multi-struct tuple + opaque generic element: `name`/`max_devices`
    // auto-match (each found in exactly one element); `id` exists in BOTH
    // License and Owner, so it must be overridden; `comment` exists in
    // neither, so the opaque src.2 covers it.
    magic_map!((db::License, db::Owner, Option<String>) => super::dtos::LicenseCard {
        id: src.1.id,
        comment: src.2,
    });
}

fn sample() -> db::License {
    db::License {
        id: Uuid::nil(),
        status: db::Status::Active,
        max_devices: 10,
        valid_until: None,
        price_usd: Some(Decimal::new(1999, 2)),
        license_type: "custom".into(),
        notes: Some("n".into()),
        tags: vec!["a".into(), "b".into()],
    }
}

#[test]
fn struct_impl_form_auto_fills_and_applies_overrides() {
    let dto: dtos::LicenseResponse = sample().map_into().unwrap();
    assert_eq!(dto.id, Uuid::nil()); // identity leaf
    assert_eq!(dto.status, dtos::StatusDto::Active); // nested enum map
    assert_eq!(dto.max_devices, 10); // identity leaf
    assert_eq!(dto.devices_used, 0); // override: absent from src
    assert!(dto.valid_until.is_none()); // Option identity
    assert_eq!(dto.price_usd, Some(19.99)); // Option<Decimal>→Option<f64>
    assert!(dto.license_type); // override: custom expr on src
    assert_eq!(dto.notes.as_deref(), Some("n")); // Option identity
    assert_eq!(dto.tags, vec!["a", "b"]); // Vec funnel
}

#[test]
fn enum_impl_form_maps_both_directions() {
    let dto: dtos::StatusDto = db::Status::Suspended.map_into().unwrap();
    assert_eq!(dto, dtos::StatusDto::Suspended);
    let back: db::Status = dto.map_into().unwrap();
    assert_eq!(back, db::Status::Suspended);
}

#[test]
fn fn_form_generates_a_plain_function() {
    assert_eq!(
        mappers::status_to_dto(db::Status::Trial).unwrap(),
        dtos::StatusDto::Trial
    );
}

#[test]
fn tuple_source_struct_plus_primitive() {
    let dto: dtos::LicenseResponse = (sample(), 7i64).map_into().unwrap();
    assert_eq!(dto.devices_used, 7); // override from opaque element
    assert_eq!(dto.status, dtos::StatusDto::Active); // auto from src.0
    assert_eq!(dto.max_devices, 10); // auto from src.0
    assert!(dto.license_type); // override referencing src.0
}

#[test]
fn tuple_source_multi_struct_with_collision_override() {
    let owner = db::Owner {
        id: Uuid::max(),
        name: "acme".into(),
    };
    let card: dtos::LicenseCard = (sample(), owner, Some("vip".to_string()))
        .map_into()
        .unwrap();
    assert_eq!(card.id, Uuid::max()); // collision resolved by override
    assert_eq!(card.name, "acme"); // auto: only Owner has `name`
    assert_eq!(card.max_devices, 10); // auto: only License has `max_devices`
    assert_eq!(card.comment.as_deref(), Some("vip")); // opaque element
}

mod sparse {
    use magic_map::MagicMap;

    #[derive(MagicMap)]
    pub struct PatchRequest {
        pub name: Option<String>,
    }

    #[derive(Debug, Default, MagicMap)]
    pub struct UpdateRow {
        pub name: Option<String>,
        pub status: Option<i32>,
        pub notes: Option<String>,
    }

    #[derive(MagicMap)]
    pub struct CreateRequest {
        pub label: String,
        pub max_devices: Option<i32>,
        pub kind: Option<super::dtos::StatusDto>,
        pub note: Option<String>,
    }

    #[derive(Debug, MagicMap)]
    pub struct CreateRow {
        pub label: String,
        pub max_devices: i32,
        pub kind: super::db::Status,
        pub note: Option<String>,
    }

    impl Default for CreateRow {
        fn default() -> Self {
            CreateRow {
                label: String::new(),
                max_devices: 15,
                kind: super::db::Status::Active,
                note: None,
            }
        }
    }
}

mod proto_like {
    use magic_map::MagicMap;

    // Mirrors a prost enum: proto3 forces a zero variant.
    #[derive(Debug, Clone, Copy, MagicMap)]
    pub enum WireSeverity {
        Unspecified,
        Info,
        Warning,
    }

    #[derive(Debug, Default, PartialEq, MagicMap)]
    pub enum RowSeverity {
        #[default]
        Info,
        Warning,
    }
}

// Enum→enum is variant-by-name per SOURCE variant; the proto3 `Unspecified`
// zero variant folds to the destination's declared default. Extra
// destination variants are fine on emit — they're simply never produced.
magic_map!(pub fn wire_severity_to_row: proto_like::WireSeverity => proto_like::RowSeverity);
magic_map!(pub fn row_severity_to_wire: proto_like::RowSeverity => proto_like::WireSeverity);

#[test]
fn enum_unspecified_folds_to_destination_default() {
    assert_eq!(
        wire_severity_to_row(proto_like::WireSeverity::Unspecified).expect("map"),
        proto_like::RowSeverity::Info,
    );
    assert_eq!(
        wire_severity_to_row(proto_like::WireSeverity::Warning).expect("map"),
        proto_like::RowSeverity::Warning,
    );
    assert!(matches!(
        row_severity_to_wire(proto_like::RowSeverity::Info).expect("map"),
        proto_like::WireSeverity::Info,
    ));
}

mod renamed {
    use magic_map::MagicMap;

    #[derive(Debug, Clone, Copy, MagicMap)]
    pub enum DbKind {
        Integer,
        Text,
        Boolean,
    }

    #[derive(Debug, PartialEq, MagicMap)]
    pub enum WireKind {
        Int32,
        String,
        Boolean,
    }
}

// Enum variant renames: explicit `Src => Dest` pairs win, the rest pair by
// name — the enum analogue of struct field overrides.
magic_map!(pub fn db_kind_to_wire: renamed::DbKind => renamed::WireKind {
    Integer => Int32,
    Text => String,
});

#[test]
fn enum_variant_renames_compose_with_name_matching() {
    assert_eq!(
        db_kind_to_wire(renamed::DbKind::Integer).expect("map"),
        renamed::WireKind::Int32,
    );
    assert_eq!(
        db_kind_to_wire(renamed::DbKind::Text).expect("map"),
        renamed::WireKind::String,
    );
    assert_eq!(
        db_kind_to_wire(renamed::DbKind::Boolean).expect("map"),
        renamed::WireKind::Boolean,
    );
}

mod derived {
    use magic_map::MagicMap;

    #[derive(MagicMap)]
    pub struct Span {
        pub start: i64,
        pub end: i64,
    }

    #[derive(Debug, MagicMap)]
    pub struct SpanStats {
        pub width: i64,
        pub midpoint: i64,
    }
}

// `let` bindings before the overrides: shared derivations that feed more
// than one destination field.
magic_map!(pub fn span_stats: derived::Span => derived::SpanStats {
    let width = src.end - src.start;
    width: width,
    midpoint: src.start + width / 2,
});

#[test]
fn let_prelude_shares_derivations_across_fields() {
    let stats = span_stats(derived::Span { start: 10, end: 20 }).expect("map");
    assert_eq!(stats.width, 10);
    assert_eq!(stats.midpoint, 15);
}

mod wrap {
    use magic_map::MagicMap;
    use uuid::Uuid;

    #[derive(MagicMap)]
    pub struct SyncSource {
        pub code: String,
        pub owner_id: String,
        pub count: i32,
        pub note: Option<String>,
    }

    #[derive(Debug, Default, MagicMap)]
    pub struct SyncRow {
        pub code: Option<String>,
        pub owner_id: Option<Uuid>,
        pub count: Option<i32>,
        pub note: Option<String>,
    }
}

// Wrap tier: plain sources land in `Option` dests as `Some(value)` (set
// semantics), funneling on the way (String→Uuid); `Option → Option` still
// goes through the plain funnel, so a `None` source stays `None` — never
// `Some`-wrapped into existence.
magic_map!(wrap::SyncSource => wrap::SyncRow {
    ..Default::default()
});

#[test]
fn defaults_trailer_wraps_plain_sources_into_option_dests() {
    let id: uuid::Uuid = "0195a8a2-1111-7000-a000-000000000001"
        .parse()
        .expect("uuid");
    let row: wrap::SyncRow = wrap::SyncSource {
        code: "EQ-1".into(),
        owner_id: id.to_string(),
        count: 3,
        note: None,
    }
    .map_into()
    .expect("wrap automap");
    assert_eq!(row.code.as_deref(), Some("EQ-1"));
    assert_eq!(row.owner_id, Some(id));
    assert_eq!(row.count, Some(3));
    assert_eq!(row.note, None);
}

#[test]
fn wrap_tier_is_strict_through_the_funnel() {
    let bad = wrap::SyncSource {
        code: "EQ-1".into(),
        owner_id: "not-a-uuid".into(),
        count: 0,
        note: None,
    };
    let res: Result<wrap::SyncRow, MappingError> = bad.map_into();
    assert!(res.is_err(), "garbage must not become Some(default)");
}

// `..Default::default()`: `name` automaps; `status`/`notes` (absent from the
// request) fall back to Default instead of erroring.
magic_map!(sparse::PatchRequest => sparse::UpdateRow {
    ..Default::default()
});

// Option<S> sources land in non-Option dests by unwrapping through the
// funnel, falling back to the DEFAULT INSTANCE's field value on None.
magic_map!(sparse::CreateRequest => sparse::CreateRow {
    ..Default::default()
});

#[test]
fn defaults_trailer_unwraps_options_with_instance_fallback() {
    let row: sparse::CreateRow = sparse::CreateRequest {
        label: "l".into(),
        max_devices: None,
        kind: Some(dtos::StatusDto::Suspended),
        note: Some("n".into()),
    }
    .map_into()
    .unwrap();
    assert_eq!(row.label, "l"); // plain funnel
    assert_eq!(row.max_devices, 15); // None -> business default from instance
    assert_eq!(row.kind, db::Status::Suspended); // Some -> funnels dto enum -> db enum
    assert_eq!(row.note.as_deref(), Some("n")); // Option -> Option untouched

    let row2: sparse::CreateRow = sparse::CreateRequest {
        label: "x".into(),
        max_devices: Some(3),
        kind: None,
        note: None,
    }
    .map_into()
    .unwrap();
    assert_eq!(row2.max_devices, 3); // Some -> unwrapped
    assert_eq!(row2.kind, db::Status::Active); // None -> default variant
    assert_eq!(row2.note, None);
}

#[test]
fn defaults_trailer_fills_missing_fields() {
    let row: sparse::UpdateRow = sparse::PatchRequest {
        name: Some("n".into()),
    }
    .map_into()
    .unwrap();
    assert_eq!(row.name.as_deref(), Some("n"));
    assert_eq!(row.status, None);
    assert_eq!(row.notes, None);
}

mod schemaless {
    // No MagicMap anywhere — usable in tuples only with full overrides.
    pub struct Untouchable {
        pub reason: String,
    }
}

// Every dest field overridden → no element schema is needed, so even
// schema-less foreign-ish types can ride along in the tuple.
magic_map!((schemaless::Untouchable, i32) => dtos::StatusDto2Holder {
    code: src.1,
    reason: src.0.reason,
});

#[test]
fn tuple_all_overridden_needs_no_schemas() {
    let h: dtos::StatusDto2Holder = (schemaless::Untouchable { reason: "r".into() }, 7)
        .map_into()
        .unwrap();
    assert_eq!(h.code, 7);
    assert_eq!(h.reason, "r");
}

mod validated {
    use magic_map::MagicMap;
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
}

magic_map!(validated::RawInput => validated::ValidatedDto);
// fn form with a validated destination.
magic_map!(pub fn raw_to_validated_dto: validated::RawInput => validated::ValidatedDto);

#[test]
fn validated_dest_passes_when_data_is_valid() {
    let dto: validated::ValidatedDto = validated::RawInput {
        name: "Alice".into(),
        email: "alice@example.com".into(),
    }
    .map_into()
    .unwrap();
    assert_eq!(dto.name, "Alice");
}

#[test]
fn validated_dest_errors_when_data_fails_constraints() {
    // Empty name violates length(min = 1).
    let result: Result<validated::ValidatedDto, _> = validated::RawInput {
        name: "".into(),
        email: "alice@example.com".into(),
    }
    .map_into();
    assert!(
        matches!(result, Err(MappingError::Validation(_))),
        "expected Validation error, got {result:?}",
    );

    // Invalid email violates #[validate(email)].
    let result2: Result<validated::ValidatedDto, _> = validated::RawInput {
        name: "Alice".into(),
        email: "not-an-email".into(),
    }
    .map_into();
    assert!(matches!(result2, Err(MappingError::Validation(_))));
}

mod validated_defaults {
    use magic_map::MagicMap;
    use validator::Validate;

    #[derive(MagicMap)]
    pub struct RawCreate {
        pub name: String,
        pub note: Option<String>,
    }

    #[derive(Debug, Validate, MagicMap)]
    pub struct NewRecord {
        #[validate(length(min = 1, max = 50))]
        pub name: String,
        pub note: Option<String>,
        pub status: String, // absent from source, filled by Default
    }

    impl Default for NewRecord {
        fn default() -> Self {
            NewRecord {
                name: String::new(),
                note: None,
                status: "new".into(),
            }
        }
    }
}

magic_map!(validated_defaults::RawCreate => validated_defaults::NewRecord {
    ..Default::default()
});

#[test]
fn validated_dest_with_defaults_trailer() {
    // Valid: status comes from Default, validation passes.
    let row: validated_defaults::NewRecord = validated_defaults::RawCreate {
        name: "Misifu".into(),
        note: None,
    }
    .map_into()
    .unwrap();
    assert_eq!(row.name, "Misifu");
    assert_eq!(row.status, "new");

    // Invalid: empty name fails length(min = 1) even after defaults are applied.
    let bad: Result<validated_defaults::NewRecord, _> = validated_defaults::RawCreate {
        name: "".into(),
        note: None,
    }
    .map_into();
    assert!(matches!(bad, Err(MappingError::Validation(_))));
}

#[test]
fn validated_dest_fn_form_validates() {
    assert!(raw_to_validated_dto(validated::RawInput {
        name: "Alice".into(),
        email: "alice@example.com".into(),
    })
    .is_ok());

    assert!(matches!(
        raw_to_validated_dto(validated::RawInput {
            name: "Alice".into(),
            email: "bad".into(),
        }),
        Err(MappingError::Validation(_))
    ));
}

mod validated_tuple {
    use magic_map::MagicMap;
    use validator::Validate;

    #[derive(MagicMap)]
    pub struct Names {
        pub name: String,
    }

    #[derive(MagicMap)]
    pub struct Contact {
        pub email: String,
    }

    #[derive(Debug, Validate, MagicMap)]
    pub struct ValidatedPerson {
        #[validate(length(min = 1, max = 50))]
        pub name: String,
        #[validate(email)]
        pub email: String,
    }
}

// Tuple source: `name` auto-matches element 0, `email` element 1, then the
// fully-built value is validated.
magic_map!(pub fn names_and_contact: (validated_tuple::Names, validated_tuple::Contact)
    => validated_tuple::ValidatedPerson);

#[test]
fn validated_dest_tuple_source_validates() {
    let ok = names_and_contact((
        validated_tuple::Names { name: "Ada".into() },
        validated_tuple::Contact {
            email: "ada@example.com".into(),
        },
    ));
    assert!(ok.is_ok());

    let bad = names_and_contact((
        validated_tuple::Names { name: "Ada".into() },
        validated_tuple::Contact {
            email: "not-an-email".into(),
        },
    ));
    assert!(matches!(bad, Err(MappingError::Validation(_))));
}

#[test]
fn leaf_conversions() {
    let u = Uuid::map_from("00000000-0000-0000-0000-000000000000".to_string()).unwrap();
    assert_eq!(u, Uuid::nil());
    assert_eq!(
        Uuid::map_from("not-a-uuid".to_string()),
        Err(MappingError::InvalidUuid { field: "<uuid>" })
    );
    let d = Decimal::map_from(20.5_f64).unwrap();
    assert_eq!(d, Decimal::new(205, 1));
    assert_eq!(
        Decimal::map_from(f64::NAN),
        Err(MappingError::OutOfRange { field: "<decimal>" })
    );
}

// ── `?` on a leaf parse error inside an override ──────────────────────────────
//
// An override expression can propagate the leaf parse errors with `?` directly,
// via `From<uuid::Error> / <chrono::ParseError> / <rust_decimal::Error> for
// MappingError`. Without those impls this needed a hand-rolled
// `.map_err(|e| MappingError::Custom(e.to_string()))`.
mod override_question_mark {
    use super::*;

    mod src {
        use magic_map::MagicMap;
        #[derive(Clone, MagicMap)]
        pub struct Row {
            pub raw_id: String,
            pub name: String,
        }
    }
    mod dst {
        use magic_map::MagicMap;
        use uuid::Uuid;
        #[derive(Debug, PartialEq, MagicMap)]
        pub struct Model {
            pub id: Uuid,
            pub name: String,
        }
    }

    // `id` is overridden and parses a String with `?`; `name` auto-maps.
    magic_map!(pub fn row_to_model: src::Row => dst::Model {
        id: Uuid::parse_str(&src.raw_id)?,
    });

    #[test]
    fn override_can_question_mark_a_uuid_parse() {
        let ok = row_to_model(src::Row {
            raw_id: "0195aaaa-0000-7000-a000-000000000001".into(),
            name: "n".into(),
        })
        .expect("valid uuid maps");
        assert_eq!(ok.name, "n");

        let err = row_to_model(src::Row {
            raw_id: "not-a-uuid".into(),
            name: "n".into(),
        })
        .unwrap_err();
        assert_eq!(
            err,
            MappingError::InvalidUuid {
                field: "<override>"
            }
        );
    }

    #[test]
    fn from_impls_cover_chrono_and_decimal() {
        let chrono_err = "nope".parse::<chrono::DateTime<chrono::Utc>>().unwrap_err();
        assert_eq!(
            MappingError::from(chrono_err),
            MappingError::Parse {
                field: "<override>"
            }
        );
        let decimal_err = "nope".parse::<Decimal>().unwrap_err();
        assert_eq!(
            MappingError::from(decimal_err),
            MappingError::Parse {
                field: "<override>"
            }
        );
    }
}

// ── foreign→foreign nested funnelling (magic_map_scope!) ─────────────────────
//
// The case the fn form could not express before: a mapper crate that owns
// neither side. `db` and `dtos` stand in for `quickedge_db` / `quickedge_dtos`
// — nothing in either knows the other exists, and no `MapFrom` impl between
// them is legal anywhere.

mod scoped {
    pub mod db {
        #[derive(magic_map::MagicMap, Clone)]
        pub struct State {
            pub name: String,
        }
        #[derive(magic_map::MagicMap, Clone)]
        pub struct Locality {
            pub name: String,
        }
        #[derive(magic_map::MagicMap, Clone)]
        pub struct PostalCode {
            pub code: String,
            pub state: State,
            pub locality: Option<Locality>,
            pub neighborhoods: Vec<Locality>,
            pub population: i32,
        }
    }

    pub mod dtos {
        #[derive(magic_map::MagicMap, Debug, PartialEq)]
        pub struct StateResponse {
            pub name: String,
        }
        #[derive(magic_map::MagicMap, Debug, PartialEq)]
        pub struct LocalityResponse {
            pub name: String,
        }
        #[derive(magic_map::MagicMap, Debug, PartialEq)]
        pub struct PostalCodeResponse {
            pub code: String,
            pub state: StateResponse,
            pub locality: Option<LocalityResponse>,
            pub neighborhoods: Vec<LocalityResponse>,
            pub population: i64, // widening leaf still funnels
        }
    }

    // A neutral "service" module: owns neither type.
    pub mod mappers {
        use super::{db, dtos};
        use magic_map::magic_map;

        magic_map!(pub fn state_to_dto: db::State => dtos::StateResponse);
        magic_map!(pub fn locality_to_dto: db::Locality => dtos::LocalityResponse);

        // Nested `State`, `Option<Locality>`, `Vec<Locality>` and the i32→i64
        // widening all auto-fill — no overrides, no hand-written recursion.
        magic_map!(pub fn postal_code_to_dto: db::PostalCode => dtos::PostalCodeResponse);
    }
}

#[test]
fn fn_form_funnels_nested_foreign_pairs() {
    use scoped::{db, dtos, mappers};

    let src = db::PostalCode {
        code: "97000".into(),
        state: db::State {
            name: "Yucatán".into(),
        },
        locality: Some(db::Locality {
            name: "Mérida".into(),
        }),
        neighborhoods: vec![
            db::Locality {
                name: "Centro".into(),
            },
            db::Locality {
                name: "Itzimná".into(),
            },
        ],
        population: 921_770,
    };

    let dto = mappers::postal_code_to_dto(src).unwrap();
    assert_eq!(dto.code, "97000");
    assert_eq!(dto.state.name, "Yucatán");
    assert_eq!(
        dto.locality,
        Some(dtos::LocalityResponse {
            name: "Mérida".into()
        })
    );
    assert_eq!(dto.neighborhoods.len(), 2);
    assert_eq!(dto.neighborhoods[0].name, "Centro");
    assert_eq!(dto.population, 921_770i64);
}

#[test]
fn fn_form_none_stays_none() {
    use scoped::{db, mappers};

    let dto = mappers::postal_code_to_dto(db::PostalCode {
        code: "00000".into(),
        state: db::State { name: "X".into() },
        locality: None,
        neighborhoods: vec![],
        population: 0,
    })
    .unwrap();
    assert_eq!(dto.locality, None);
    assert!(dto.neighborhoods.is_empty());
}

/// Leaves are found without being declared. Tier 1 of the probe holds only
/// the concrete pairs the fn form emitted, so none of these has a candidate
/// there and every one falls through to the `MapFrom` blanket. A regression
/// that made tier 1 blanket-shaped would match these structurally and fail on
/// the bound instead — which is exactly the bug this guards.
#[test]
fn leaves_resolve_through_the_probe_without_registration() {
    use __magic_map_scope::{ProbeGlobal as _, ProbeLocal as _};
    use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};

    macro_rules! probe {
        ($src:expr, $ty:ty) => {{
            let out: $ty = (&mut &mut magic_map::MapProbe::new($src))
                .magic_probe()
                .expect("leaf must resolve without registration");
            out
        }};
    }

    probe!(true, bool);
    probe!('x', char);
    probe!(1i8, i8);
    probe!(1i16, i16);
    probe!(1i32, i32);
    probe!(1i64, i64);
    probe!(1i128, i128);
    probe!(1isize, isize);
    probe!(1u8, u8);
    probe!(1u16, u16);
    probe!(1u32, u32);
    probe!(1u64, u64);
    probe!(1u128, u128);
    probe!(1usize, usize);
    probe!(1f32, f32);
    probe!(1f64, f64);
    probe!(String::from("s"), String);

    // widenings
    probe!(1u8, u16);
    probe!(1i32, i64);
    probe!(1f32, f64);

    // feature leaves
    let id = Uuid::from_u128(0x0192_3f4b_5c6d_7e8f_9012_3456_789a_bcde);
    probe!(id, Uuid);
    probe!(id, String);
    probe!(id.to_string(), Uuid);

    let d = Decimal::new(1995, 2);
    probe!(d, Decimal);
    probe!(d, f64);
    probe!(d, String);
    probe!(String::from("19.95"), Decimal);
    probe!(19.95f64, Decimal);

    let now: DateTime<Utc> = Utc::now();
    probe!(now, DateTime<Utc>);
    probe!(now, String);
    probe!(now.to_rfc3339(), DateTime<Utc>);
    let day = NaiveDate::from_ymd_opt(2026, 8, 9).unwrap();
    probe!(day, NaiveDate);
    probe!(day, String);
    probe!(day.to_string(), NaiveDate);
    probe!(now.naive_utc(), NaiveDateTime);
    probe!(now.time(), NaiveTime);

    probe!(serde_json::Value::Null, serde_json::Value);

    // wrappers over leaves, still tier 2
    probe!(vec![1i32, 2i32], Vec<i64>);
    probe!(Some(id), Option<String>);

    // …and a declared pair resolves through tier 1 in the very same scope, so
    // the fallthrough above is a real fallthrough, not tier 1 being absent.
    let state: scoped::dtos::StateResponse =
        (&mut &mut magic_map::MapProbe::new(scoped::db::State {
            name: "Yucatán".into(),
        }))
            .magic_probe()
            .expect("declared pair must resolve through tier 1");
    assert_eq!(state.name, "Yucatán");
}

// ── custom leaves ───────────────────────────────────────────────────────────

mod custom_leaf {
    /// A leaf declared the normal way, in the crate that owns it: `MapFrom`
    /// impls the local trait cannot see until `leaves: [...]` names them.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Celsius(pub i32);

    magic_map::map_identity!(Celsius);

    /// A generic wrapper, like `quickedge_commons::Patch<T>`: its `MapFrom`
    /// impl is generic, so no list of concrete pairs can cover it.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Wrap<T>(pub T);

    impl<S, D: magic_map::MapFrom<S>> magic_map::MapFrom<Wrap<S>> for Wrap<D> {
        fn map_from(src: Wrap<S>) -> Result<Self, magic_map::MappingError> {
            Ok(Wrap(D::map_from(src.0)?))
        }
    }

    impl magic_map::MapFrom<Celsius> for String {
        fn map_from(src: Celsius) -> Result<Self, magic_map::MappingError> {
            Ok(format!("{}C", src.0))
        }
    }
}

mod custom_leaf_use {
    use super::custom_leaf::Celsius;
    use magic_map::magic_map;

    pub mod db {
        #[derive(magic_map::MagicMap, Clone)]
        pub struct Reading {
            pub at: String,
            pub temp: super::Celsius,
            pub note: crate::custom_leaf::Wrap<i32>,
        }
    }
    pub mod dtos {
        #[derive(magic_map::MagicMap, Debug, PartialEq)]
        pub struct ReadingResponse {
            pub at: String,
            pub temp: String, // Celsius → String, via the declared leaf
            pub note: crate::custom_leaf::Wrap<i64>, // generic leaf + widening
        }
    }

    magic_map!(pub fn reading_to_dto: db::Reading => dtos::ReadingResponse);
}

#[test]
fn custom_and_generic_leaves_need_no_declaration() {
    let dto = custom_leaf_use::reading_to_dto(custom_leaf_use::db::Reading {
        at: "2026-08-09".into(),
        temp: custom_leaf::Celsius(21),
        note: custom_leaf::Wrap(7i32),
    })
    .unwrap();
    assert_eq!(dto.temp, "21C");
    assert_eq!(dto.note, custom_leaf::Wrap(7i64));
}
