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
