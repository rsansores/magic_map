# Migrating 0.3 → 0.4

One rename, three new capabilities. The rename is mechanical — a `sed` and a
build — and it is the only thing that breaks.

---

## Why

### The `Result` nobody could get rid of

`MapFrom::map_from` returned `Result<Self, MappingError>` because *some*
conversions can fail. `String` → `Uuid` parses. `f64` → `Decimal` can be NaN.

But an identity is not one of those, and neither is a lossless widening, and
neither is `Uuid` → `String`. A mapping built entirely from those cannot fail,
and 0.3 had no way to say so. Every call site wrote `?` for a failure that did
not exist, and — worse — a function with no other fallible step had to start
returning `Result` just to host the `?`.

A one-field copy between two structs is the clearest case. In 0.3 the honest
options were an infallible hand-written `impl From` (giving up the funnel, the
leaf conversions, and every guarantee the crate exists to provide) or a `?` on
a `Uuid` move. Neither is right, so people picked `From`, and the whole point
of the crate leaked away one struct at a time.

0.4 splits the two:

| | infallible | fallible |
|---|---|---|
| trait | `MapFrom` / `MapInto` | `TryMapFrom` / `TryMapInto` |
| method | `map_from` / `map_into` | `try_map_from` / `try_map_into` |
| returns | `Self` | `Result<Self, MappingError>` |
| declared | `magic_map!(infallible …)` | `magic_map!(…)` |

The names now line up with `From` / `TryFrom`, which is what they should have
been at 0.1.

**The claim is checked, not trusted.** An infallible expansion contains no `?`,
so a field pair that only has a `TryMapFrom` route fails to resolve:

```rust
magic_map!(infallible StringId => UuidId);
```
```
error[E0277]: the trait bound `Uuid: MapFrom<String>` is not satisfied
```

You cannot claim infallible for something that can fail. And infallible
mappings get the fallible half generated too, so `try_map_into()` still works
on them — a caller that does not care stays unaware.

### Borrowed sources

The tuple form always accepted references in element position; only the
top-level parse refused them. That left a borrowed source spelled as a
one-tuple, `f((payload,))?`, or a clone of a payload to read three fields off
it. Now:

```rust
magic_map!(infallible fn fiscal_fields: &CompanyPayload => FiscalFields { … });
```

### Sealing — `#[mapped(sealed)]`

The failure this crate exists to prevent is not a wrong `From` impl. It is the
field-by-field copy someone types directly into a service or a controller,
using no trait at all. No linter catches that reliably: a grep cannot tell
construction from destructuring, and cannot tell a mapping from assembling a
page envelope.

`#[mapped(sealed)]` adds `#[non_exhaustive]` and a hidden all-fields
constructor. From any other crate the type then has **no struct expression at
all**, so the hand-rolled copy stops compiling:

```
error[E0639]: cannot create non-exhaustive struct using struct expression
```

`magic_map!` builds through the constructor and keeps working, so declared
mappings are unaffected.

**Banning `impl From` needs no separate feature.** A `From` impl for a sealed
type cannot construct its own output — the impl body hits the same E0639. Forbid
the manual map and you have forbidden the manual `From` with it.

It is an attribute rather than a derive because a derive is additive-only: it
sees the item and emits new items beside it, and can never place
`#[non_exhaustive]` *on* it. It is spelled `#[mapped]` rather than
`#[magic_map]` because the latter collides with the `magic_map!` macro —
function-like and attribute macros share a namespace (`E0428`).

---

## What sealing does and does not reach

Worth knowing before rolling it out, because the boundary is the *crate*, not
the directory.

**Reached** — any type whose owning crate is not the crate doing the mapping.
In a layered codebase that is most of them: wire DTOs, database models, and
generated proto types are each owned by their own crate, and the mappers live
somewhere else.

**Not reached:**

- **Types local to the mapping crate.** `#[non_exhaustive]` has no effect
  within the defining crate, so a helper struct declared next to the mapper is
  not protected. Moving code between modules changes nothing — `services/` and
  `mappers/` in one crate are the same crate to the compiler.
- **Conversions *out of* a sealed type.** `impl From<Sealed> for Other`
  compiles: the sealed type is the source, and nothing is being constructed.
- **Anything not sealed.** Sealing is per-type and opt-in.

So sealing is a strong guarantee at layer boundaries and silent inside a layer.
Keep whatever review or lint you use for the rest.

**Do not seal blanket-style.** A field-less marker — a proto `Empty` — gains
nothing and breaks every `Ok(Empty {})` in the tree; the attribute skips
zero-field structs, unit and tuple structs, and enums for that reason. Test
fixtures in other crates construct types by hand and will break: decide how
they build values before you seal a widely-used type.

---

## Doing it

### 1. Rename (required)

Every 0.3 name was the fallible one:

```sh
git ls-files -z '*.rs' '*.md' | xargs -0 sed -i \
  's/\bMapFrom\b/TryMapFrom/g;  s/\bmap_from\b/try_map_from/g;
   s/\bMapInto\b/TryMapInto/g;  s/\bmap_into\b/try_map_into/g'
```

Word boundaries matter: they leave `map_identity!`, `map_display!`,
`map_parse!` and `magic_map_scope!` alone. Re-exports move with everything
else. Then build — there is nothing else to do.

### 2. Make the conversions that cannot fail say so (optional)

Add `infallible` and drop the `?`:

```rust
magic_map!(infallible ExtractorContext => commons::TenantContext);
```
```rust
let scope = tenant.map_into();   // was: tenant.into(), or try_map_into()?
```

If it does not compile, the mapping was not infallible and the error names the
field pair.

### 3. Seal, one owning crate at a time (optional)

Replace the derive — the attribute must come **first**, since it rewrites the
item the derives then see:

```rust
#[mapped(sealed)]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CustomerDto { … }
```

For prost, one line, and prefer a path over `"."`:

```rust
.type_attribute(".mypkg.Customer", "#[magic_map::mapped(sealed)]")
```

`#[mapped]` with no argument is exactly `#[derive(MagicMap)]`; the derive stays
supported, so this is per-type and can stop wherever you want.

Expect a first pass to fail on real findings, not on the mechanism. Fix them by
declaring the mapping rather than by reaching for
`__magic_map_new_unchecked` — which is public only because a macro expansion
holds no privilege a hand-written line lacks, and which is named to be obvious
in review.
