//! Sealing, exercised the only way that means anything: from another crate.
//! `leaf_provider` owns the types; this crate is foreign to them, exactly as a
//! service crate is foreign to the crate that owns its DTOs.
use leaf_provider::{OpenDto, SealedDto};
// The schema alias lives next to the type, so the destination is spelled by path.
use magic_map::{magic_map, MagicMap, TryMapInto};

magic_map::magic_map_scope!(from: [leaf_provider]);

#[derive(MagicMap)]
pub struct Row {
    pub id: String,
    pub count: u64,
}

// The generated mapping targets a sealed type and compiles: it builds through
// the constructor rather than a struct expression.
magic_map!(Row => leaf_provider::SealedDto);

#[test]
fn magic_map_can_still_build_a_sealed_type() {
    let d: SealedDto = Row {
        id: "x".into(),
        count: 2,
    }
    .try_map_into()
    .unwrap();
    assert_eq!(d, SealedDto::__magic_map_new_unchecked("x".into(), 2));
}

#[test]
fn an_unsealed_type_is_unaffected() {
    // The control: still constructible by hand from here.
    let d = OpenDto {
        id: "x".into(),
        count: 2,
    };
    assert_eq!(d.count, 2);
}

// ── what sealing forbids, from a foreign crate ───────────────────────────────
// Both of these are E0639, "cannot create non-exhaustive struct using struct
// expression". They are the reason the attribute exists, so they are written
// down rather than merely believed:
//
//     fn hand_rolled_map(r: Row) -> SealedDto {
//         SealedDto { id: r.id, count: r.count }   // E0639
//     }
//
//     impl From<Local> for SealedDto {
//         fn from(l: Local) -> Self {
//             Self { id: l.0, count: 0 }           // E0639
//         }
//     }
//
// The second is the one worth noticing: banning `impl From` needs no separate
// mechanism, because a From impl for a sealed type cannot construct its own
// output. Forbidding the hand-rolled map forbids the hand-rolled From with it.

// ── #[mapped(sealed, patch)] ─────────────────────────────────────────────────
// The state-transition case: a sparse update built from nothing, in a crate
// that cannot write the struct expression for it.

#[test]
fn patch_starts_empty_and_chains_only_what_it_touches() {
    let p = leaf_provider::SealedPatch::patch()
        .count(7)
        .note(Some("sold".into()));
    assert_eq!(p.count, 7);
    assert_eq!(p.note.as_deref(), Some("sold"));
    // Untouched fields keep the model's Default — the point of a sparse patch.
    assert_eq!(p.id, "");
}

#[test]
fn patch_is_empty_by_default() {
    assert_eq!(
        leaf_provider::SealedPatch::patch(),
        leaf_provider::SealedPatch::default()
    );
}

#[test]
fn setters_are_last_write_wins() {
    let p = leaf_provider::SealedPatch::patch().count(1).count(2);
    assert_eq!(p.count, 2);
}

// A sealed+patch type is still a legal magic_map destination: the declared
// mapping goes through the hidden constructor, not the setters.
magic_map!(Row => leaf_provider::SealedPatch { note: None });

#[test]
fn patch_does_not_disturb_declared_mappings() {
    let p: leaf_provider::SealedPatch = Row {
        id: "x".into(),
        count: 2,
    }
    .try_map_into()
    .unwrap();
    assert_eq!(
        p,
        leaf_provider::SealedPatch::patch().id("x".into()).count(2)
    );
}

// Still sealed. This remains E0639 from here, `patch` or no `patch`:
//
//     leaf_provider::SealedPatch { id: "x".into(), count: 2, note: None }
