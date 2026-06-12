//! The mapping machinery is tested ONCE, here — against synthetic types that
//! mirror the db/dtos/mappers layering. Real mappers built on `magic_map!`
//! don't carry their own unit tests: asserting a generated field-by-field copy
//! is restating the declaration.

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
