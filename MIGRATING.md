# Migrating 0.4 → 0.5

One removal, mechanical, and it exists to stop one thing: a mapping that
compiles without anyone having decided what its unmapped fields mean.

---

## `..Default::default()` is gone — say which defaults you mean

The old trailer covered two unrelated situations with one spelling:

- a destination field with a **real business default** (`created_at`, a status
  that starts at `pending`), and
- a field **nobody has mapped yet**, where `Default` is a placeholder standing
  in for work that has not been done.

Both compiled. Neither was distinguishable in review, and only the first one
should survive a schema change: add a column and the old trailer silently
absorbed it, which is exactly the guarantee a declaration-site mapper exists to
provide.

0.5 replaces it with two trailers:

| trailer | a field may be omitted when… |
|---|---|
| *(none)* | never — every destination field must be mapped or overridden |
| `..DeclaredDefaults` | the model declares its own `#[default(..)]` for it |
| `..AnyDefault` | always — whatever `Default` gives is accepted |

```rust
// 0.4
magic_map!(api::CreateCatRequest => db::CreateCat { ..Default::default() });

// 0.5 — the model says what the unmapped fields mean
magic_map!(api::CreateCatRequest => db::CreateCat { ..DeclaredDefaults });
```
```rust
#[derive(SmartDefault, magic_map::MagicMap)]
pub struct CreateCat {
    pub name: String,
    #[default = "new"]        // ← what `..DeclaredDefaults` reads
    pub status: String,
}
```

`#[default(..)]` is read as a plain attribute, so any derive using that
spelling works. magic_map takes no dependency on one.

### Migrating

The cheap path is two steps, and the first one is a `sed`:

1. **`..Default::default()` → `..AnyDefault` everywhere.** Behaviour is
   identical, so the build goes green immediately.
2. **Promote to `..DeclaredDefaults` crate by crate.** Each one fails with the
   fields it cannot account for, named. Map them, or give the model a
   `#[default(..)]`, or leave that mapping on `..AnyDefault` and come back.

Step 2 is worth doing per crate rather than all at once: the compiler's list is
the inventory of every mapping nobody finished, and it is easier to act on a
crate at a time.

Leaving a mapping on `..AnyDefault` forever is a legitimate answer in exactly
two cases — a **patch model**, whose contract already is "a field absent from
the request is a column left untouched", and a **mapping still under
construction**, where it is the honest marker rather than a silenced error.

---

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

One dependency dividend: `magic_map_scope!` no longer expands `::uuid::Uuid`
and friends into *your* crate — the leaf groups resolve through magic_map's
own re-exports. A crate that only kept `uuid`, `chrono`, `rust_decimal` or
`serde_json` as direct dependencies to satisfy the scope can drop them.

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

Infallible fn-forms compose: `magic_map_scope!` plants an infallible local
funnel beside the fallible one, so an `infallible fn` mapping nests another
foreign→foreign `infallible fn` mapping the same way fallible ones always
nested.

**Mark your leaves.** `map_identity!` and `map_display!` now emit the
`MapFrom` twin automatically (an identity or a `Display` cannot fail). A
hand-written custom leaf that cannot fail writes one `MapFrom` impl and takes
an `infallible` prefix in the `magic_map_leaves!` block — that one impl then
backs both funnels, local and global:

```rust
magic_map::magic_map_leaves! {
    identity: [crate::FileKind],
    custom: [
        infallible crate::TimeZone => String,  // impl MapFrom<TimeZone> for String
        String => crate::TimeZone,             // parse: stays fallible
    ],
}
```

A pair left unmarked keeps working fallibly — marking is what lets it appear
inside `infallible` mappings.

### 3. Seal, one owning crate at a time (optional)

Replace the derive — the attribute must come **first**, since it rewrites the
item the derives then see:

```rust
#[mapped(sealed)]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CustomerDto { … }
```

For prost, one line per package, and prefer a path over `"."` — seal the
model packages (mapping destinations), keep request/args packages on plain
`#[magic_map::mapped]` (clients build those from local state, which is
parameter construction, not mapping):

```rust
.type_attribute(".mypkg.models", "#[magic_map::mapped(sealed)]")
.type_attribute(".mypkg.rpc", "#[magic_map::mapped]")
```

An export disambiguation rides the attribute itself —
`#[magic_map::mapped(sealed, export = "PkgASale")]` — or the old
`#[magic_map(export = "…")]` helper next to it; both still work.

`#[mapped]` with no argument is exactly `#[derive(MagicMap)]`; the derive stays
supported, so this is per-type and can stop wherever you want.

Expect a first pass to fail on real findings, not on the mechanism. Most of
what it flags falls into two buckets:

- **A real hand-rolled mapping** — declare it. Do not reach for
  `__magic_map_new_unchecked`, which is public only because a macro expansion
  holds no privilege a hand-written line lacks, and which is named to be
  obvious in review.
- **Parameter construction** — query/filter structs, seed and test fixtures,
  sparse patches, page envelopes: built from local arguments, with no source
  type to map from. Forcing a declaration onto these puts a hand-built struct
  next to the hand-built struct it replaced. Leave them on plain `#[mapped]`.
  Context-like single-field types can keep the seal by growing an ordinary
  constructor instead.

Destructuring a sealed type in a pattern needs a trailing `..` —
`#[non_exhaustive]` blocks exhaustive patterns along with construction.

### 4. Lint the escape hatch shut (optional)

Sealing cannot reach types local to the mapping crate, conversions *out of* a
sealed type, or anything unsealed. `magic-map-lint` covers those: it walks a
source tree and flags every `impl From / Into / TryFrom / TryInto`, which is
the escape hatch that grows back. Error conversions (`impl From<MappingError>
for ApiError` — either side's name ending in `Error`) are exempt, and an
allowlist file holds the cases that are genuinely not mappings; a stale
allowlist entry fails the run, so the list only shrinks.

```sh
cargo install magic_map_lint
magic-map-lint --allow .magic-map-allow src/ crates/
```
