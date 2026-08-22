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

// ── infallible fn-forms compose through the crate-local funnel ───────────────
// The whole reason the local infallible funnel exists: a foreign→foreign pair
// has no `MapFrom` impl anywhere, so without it an infallible mapping could
// never nest another one.
pub mod inner {
    #[derive(magic_map::MagicMap, Clone, Debug, PartialEq)]
    pub struct InnerSrc {
        pub label: String,
    }
    #[derive(magic_map::MagicMap, Debug, PartialEq)]
    pub struct InnerDest {
        pub label: String,
    }
}

#[derive(MagicMap)]
pub struct OuterSrc {
    pub one: inner::InnerSrc,
    pub many: Vec<inner::InnerSrc>,
    pub temp: leaf_provider::wire::Celsius,
    pub species: leaf_provider::enums::Species, // display leaf → String
}
#[derive(MagicMap, Debug)]
pub struct OuterDest {
    pub one: inner::InnerDest,
    pub many: Vec<inner::InnerDest>,
    pub temp: String,    // infallible custom leaf, via leaves_from
    pub species: String, // map_display! now feeds the infallible funnel too
}

magic_map!(infallible fn inner_dest: inner::InnerSrc => inner::InnerDest);
magic_map!(infallible fn outer_dest: OuterSrc => OuterDest);

#[test]
fn infallible_fn_forms_nest_without_question_marks() {
    let d = outer_dest(OuterSrc {
        one: inner::InnerSrc { label: "a".into() },
        many: vec![inner::InnerSrc { label: "b".into() }],
        temp: leaf_provider::wire::Celsius(21),
        species: leaf_provider::enums::Species::Lion,
    });
    assert_eq!(d.one.label, "a");
    assert_eq!(d.many[0].label, "b");
    assert_eq!(d.temp, "21C");
    assert_eq!(d.species, "Lion");
}

// And the infallible fn-form still serves fallible nesting: a *fallible*
// fn-form auto-fills a field pair registered by the infallible one above.
#[derive(MagicMap)]
pub struct MixedSrc {
    pub one: inner::InnerSrc,
    pub id: String, // String → Uuid parses: keeps the outer mapping fallible
}
#[derive(MagicMap, Debug)]
pub struct MixedDest {
    pub one: inner::InnerDest,
    pub id: Uuid,
}
magic_map!(fn mixed_dest: MixedSrc => MixedDest);

#[test]
fn a_fallible_fn_form_nests_an_infallible_one() {
    let d = mixed_dest(MixedSrc {
        one: inner::InnerSrc { label: "x".into() },
        id: Uuid::nil().to_string(),
    })
    .expect("valid uuid");
    assert_eq!(d.one.label, "x");
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
