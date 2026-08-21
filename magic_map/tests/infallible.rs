//! `infallible` mappings, reference sources, and the check that you cannot
//! claim infallible for a conversion that can fail.
use magic_map::{magic_map, MagicMap, MapInto, TryMapInto};
use uuid::Uuid;

// The fn form registers itself in the crate-local funnel, so the scope has to
// exist here as it does in any crate that uses it.
magic_map::magic_map_scope!(from: [leaf_provider]);

#[derive(MagicMap, Clone)]
pub struct Src {
    pub id: Uuid,
    pub name: String,
    pub count: u32,
}

#[derive(MagicMap, Debug, PartialEq)]
pub struct Dest {
    pub id: String, // Uuid -> String is a to_string(): infallible
    pub name: String,
    pub count: u64, // u32 -> u64 is a lossless widening: infallible
}

magic_map!(infallible Src => Dest);

#[test]
fn infallible_impl_form_needs_no_question_mark() {
    let id = Uuid::nil();
    let d: Dest = Src {
        id,
        name: "a".into(),
        count: 7,
    }
    .map_into();
    assert_eq!(d.id, id.to_string());
    assert_eq!(d.count, 7u64);
}

#[test]
fn infallible_is_also_reachable_fallibly() {
    // The generated TryMapFrom half: a fallible caller does not need to know.
    let d: Dest = Src {
        id: Uuid::nil(),
        name: "a".into(),
        count: 1,
    }
    .try_map_into()
    .expect("infallible mapping cannot fail");
    assert_eq!(d.count, 1u64);
}

#[derive(MagicMap)]
pub struct RefDest {
    pub name: String,
    pub count: u64,
}

// A borrowed source — the whole reason this exists is that cloning a large
// payload to map three fields off it is not a trade anyone wants to make.
magic_map!(infallible fn ref_dest: &Src => RefDest {
    name: src.name.clone(),
    count: src.count as u64,
});

#[test]
fn reference_source_maps_without_moving() {
    let s = Src {
        id: Uuid::nil(),
        name: "borrowed".into(),
        count: 3,
    };
    let d = ref_dest(&s);
    assert_eq!(d.name, "borrowed");
    assert_eq!(s.name, "borrowed"); // still ours
}

// ── enums: variant-to-variant over unit enums cannot fail ────────────────────
#[derive(MagicMap, Debug, PartialEq)]
pub enum WireKind {
    Image,
    Document,
}
#[derive(MagicMap, Debug, PartialEq)]
pub enum DomainKind {
    Image,
    Document,
}
magic_map!(infallible WireKind => DomainKind);

#[test]
fn infallible_enum_form_needs_no_question_mark() {
    let d: DomainKind = WireKind::Document.map_into();
    assert_eq!(d, DomainKind::Document);
    // and the fallible half still comes free
    let d: DomainKind = WireKind::Image.try_map_into().expect("cannot fail");
    assert_eq!(d, DomainKind::Image);
}

// ── the claim is not on the honour system ────────────────────────────────────
// String -> Uuid parses, so it has a TryMapFrom route and no MapFrom one.
// Uncommenting this must not compile.
#[derive(MagicMap)]
pub struct StringId {
    pub id: String,
}
#[derive(MagicMap)]
pub struct UuidId {
    pub id: Uuid,
}
magic_map!(StringId => UuidId); // fallible: fine
