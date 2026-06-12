//! `magic_map!` — declarative struct/enum mapping, declared at the CALL SITE
//! (your mappers module), not on the type. See the proc-macro entry points in `lib.rs`
//! for the user-facing docs.
//!
//! Mechanism, in three steps:
//!
//!   1. `#[derive(MagicMap)]` — a CONTENT-FREE metadata derive. It knows
//!      nothing about any other layer; it only re-publishes the type's own
//!      field (or variant) names as a hidden macro (`__magic_map_schema_<Type>`)
//!      sitting next to the type. This is the only thing a model/dto/proto
//!      type ever carries.
//!   2. `magic_map!(Src => Dest { overrides })` — written at the call site. A
//!      function-like macro cannot see `Dest`'s fields, so it expands to a call
//!      of `Dest`'s schema macro (rewriting the last path segment), which…
//!   3. …calls back into `__magic_map_expand!` with the field list spliced in,
//!      which generates the real `impl MapFrom<Src> for Dest` — or a plain
//!      `fn` for the foreign→foreign case (e.g. db→proto in a service crate)
//!      where the orphan rule forbids any impl.
//!
//! Every non-overridden destination field is pulled from `src.<same name>`
//! through the `magic_map::MapFrom` leaf funnel, so String→Uuid,
//! Decimal↔f64, Option/Vec wrappers and mapped enums compose for free.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Data, DeriveInput, Fields, Ident, Path, Token};

// ── 1. #[derive(MagicMap)] — metadata only ───────────────────────────────────

pub fn derive(input: DeriveInput) -> Result<TokenStream, syn::Error> {
    let name = &input.ident;
    let schema_name = format_ident!("__magic_map_schema_{}", name);

    // Unsupported shapes (tuple/unit structs, enums with payload variants,
    // unions) are a silent no-op rather than an error, so the derive can be
    // applied blanket-style — e.g. prost-build's `type_attribute(".", ..)`
    // over a whole proto tree. Using such a type as a magic_map! destination
    // fails with "cannot find macro `__magic_map_schema_<T>`".
    let shape = match &input.data {
        Data::Struct(d) => match &d.fields {
            Fields::Named(named) => {
                let fields = named.named.iter().map(|f| f.ident.as_ref().unwrap());
                quote! { @struct [ #(#fields),* ] }
            }
            _ => return Ok(TokenStream::new()),
        },
        Data::Enum(d) => {
            if d.variants.iter().any(|v| !matches!(v.fields, Fields::Unit)) {
                return Ok(TokenStream::new());
            }
            let variants = d.variants.iter().map(|v| &v.ident);
            quote! { @enum [ #(#variants),* ] }
        }
        Data::Union(_) => return Ok(TokenStream::new()),
    };

    // `#[magic_map(export = "UniqueName")]` overrides only the crate-root
    // export name — needed when two same-named types live in one crate (e.g.
    // a proto crate with `g4s.Sale` and `models.Sale`). The module-scoped
    // alias that magic_map! resolves keeps the real type name either way.
    let mut export_override: Option<String> = None;
    for attr in &input.attrs {
        if attr.path().is_ident("magic_map") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("export") {
                    let v: syn::LitStr = meta.value()?.parse()?;
                    export_override = Some(v.value());
                    Ok(())
                } else {
                    Err(meta.error("expected `export = \"UniqueName\"`"))
                }
            })?;
        }
    }

    // Two-step export dance (the `macro_vis` trick): #[macro_export] is the
    // only stable way to make a macro_rules reachable from another crate, but
    // a macro-expanded export can't be referenced by absolute path within its
    // own crate (rust#52234). Exporting under an internal name and `pub use`-
    // aliasing it next to the type gives a normal module-scoped item, which
    // any path — relative, crate::, or cross-crate — reaches fine.
    // Consequence of the crate-root export: two same-named MagicMap types in
    // one crate collide — rename one or skip the derive on it.
    let export_name = match &export_override {
        Some(unique) => format_ident!("__magic_map_export_schema_{}", unique),
        None => format_ident!("__magic_map_export_schema_{}", name),
    };
    Ok(quote! {
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #export_name {
            ($($rest:tt)*) => {
                ::magic_map::__magic_map_expand! { #shape $($rest)* }
            };
        }
        // allow(unused_imports): the alias is only consumed when the type is
        // a magic_map! destination; source-only types must not warn.
        #[doc(hidden)]
        #[allow(unused_imports)]
        pub use #export_name as #schema_name;
    }
    .into())
}

// ── 2. magic_map! — the call-site front end ──────────────────────────────────

pub struct MagicMapInput {
    func: Option<(syn::Visibility, Ident)>,
    src: syn::Type,
    dest: Path,
    overrides: TokenStream2,
}

impl Parse for MagicMapInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let func = if input.peek(Token![pub]) || input.peek(Token![fn]) {
            let vis: syn::Visibility = input.parse()?;
            input.parse::<Token![fn]>()?;
            let name: Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            Some((vis, name))
        } else {
            None
        };
        let src: syn::Type = input.parse()?;
        match &src {
            syn::Type::Path(_) => {}
            syn::Type::Tuple(t) if !t.elems.is_empty() => {}
            _ => {
                return Err(syn::Error::new_spanned(
                    &src,
                    "magic_map! source must be a type path or a non-empty tuple of types",
                ))
            }
        }
        input.parse::<Token![=>]>()?;
        let dest: Path = input.parse()?;
        let overrides = if input.peek(syn::token::Brace) {
            let content;
            syn::braced!(content in input);
            content.parse::<TokenStream2>()?
        } else {
            TokenStream2::new()
        };
        Ok(Self {
            func,
            src,
            dest,
            overrides,
        })
    }
}

pub fn front(input: MagicMapInput) -> TokenStream {
    let MagicMapInput {
        func,
        src,
        dest,
        overrides,
    } = input;

    // The schema macro alias lives next to Dest: rewrite the last path
    // segment, a::b::Dest → a::b::__magic_map_schema_Dest.
    let mut schema = dest.clone();
    let last = schema.segments.last_mut().unwrap();
    last.ident = format_ident!("__magic_map_schema_{}", last.ident);

    let mode = match func {
        Some((vis, name)) => quote! { @fn(#vis #name) },
        None => quote! { @impl },
    };

    quote! {
        #schema! { #mode @src(#src) @dest(#dest) @overrides{ #overrides } }
    }
    .into()
}

// ── 3. __magic_map_expand! — the back end (schema macro calls back here) ─────
//
// Tuple sources need the FIELD LISTS of their struct elements, which only
// those elements' own schema macros know. The expander therefore runs as a
// fixpoint: while some open tuple element's schema is missing, it re-emits its
// entire input wrapped in that element's schema macro (tagged `@for(i)`),
// which prepends the shape and calls back here. Input grammar per pass:
//
//   ( @struct|@enum [names] @for(idx) )*   — collected source element shapes
//   @struct|@enum [names]                  — destination shape
//   @impl | @fn(vis name)
//   @src(Type) @dest(Path) @overrides{ field: expr, .. }

struct Shape {
    is_enum: bool,
    names: Vec<Ident>,
}

fn parse_shape(input: ParseStream) -> syn::Result<Shape> {
    use syn::ext::IdentExt;
    input.parse::<Token![@]>()?;
    let kind: Ident = input.call(Ident::parse_any)?;
    let names_content;
    syn::bracketed!(names_content in input);
    let names = Punctuated::<Ident, Token![,]>::parse_terminated(&names_content)?
        .into_iter()
        .collect();
    Ok(Shape {
        is_enum: kind == "enum",
        names,
    })
}

pub struct ExpandInput {
    collected: Vec<(usize, Shape)>,
    dest_shape: Shape,
    func: Option<(syn::Visibility, Ident)>,
    src: syn::Type,
    dest: Path,
    overrides: Vec<(Ident, syn::Expr)>,
    /// `let` bindings written before the overrides — shared derivations that
    /// feed more than one destination field.
    lets: Vec<syn::Stmt>,
    /// Enum mappings: `SrcVariant => DestVariant` rename pairs.
    variant_renames: Vec<(Ident, Ident)>,
    defaults: bool,
}

impl Parse for ExpandInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        use syn::ext::IdentExt;

        // Leading shapes: each one tagged `@for(idx)` is a collected source
        // element shape; the first untagged one is the destination's.
        let mut collected = Vec::new();
        let dest_shape = loop {
            let shape = parse_shape(input)?;
            let fork = input.fork();
            let tagged = fork.parse::<Token![@]>().is_ok()
                && fork.call(Ident::parse_any).is_ok_and(|i| i == "for");
            if tagged {
                input.parse::<Token![@]>()?;
                input.call(Ident::parse_any)?; // for
                let idx_content;
                syn::parenthesized!(idx_content in input);
                let idx: syn::LitInt = idx_content.parse()?;
                collected.push((idx.base10_parse::<usize>()?, shape));
            } else {
                break shape;
            }
        };

        // @impl | @fn(vis name)
        input.parse::<Token![@]>()?;
        let mode: Ident = input.call(Ident::parse_any)?;
        let func = if mode == "fn" {
            let fn_content;
            syn::parenthesized!(fn_content in input);
            let vis: syn::Visibility = fn_content.parse()?;
            let name: Ident = fn_content.parse()?;
            Some((vis, name))
        } else {
            None
        };

        // @src(Type) @dest(Path)
        input.parse::<Token![@]>()?;
        input.parse::<Ident>()?; // src
        let src_content;
        syn::parenthesized!(src_content in input);
        let src: syn::Type = src_content.parse()?;

        input.parse::<Token![@]>()?;
        input.parse::<Ident>()?; // dest
        let dest_content;
        syn::parenthesized!(dest_content in input);
        let dest: Path = dest_content.parse()?;

        // @overrides{ field: expr, .. }
        input.parse::<Token![@]>()?;
        input.parse::<Ident>()?; // overrides
        let ov_content;
        syn::braced!(ov_content in input);
        let mut overrides = Vec::new();
        let mut lets = Vec::new();
        let mut variant_renames = Vec::new();
        let mut defaults = false;
        while !ov_content.is_empty() {
            if ov_content.peek(Token![let]) {
                lets.push(ov_content.parse::<syn::Stmt>()?);
                continue;
            }
            // Trailing `..Default::default()`: dest fields absent from the
            // source fall back to Default instead of erroring.
            if ov_content.peek(Token![..]) {
                ov_content.parse::<Token![..]>()?;
                let expr: syn::Expr = ov_content.parse()?;
                if quote!(#expr).to_string().replace(' ', "") != "Default::default()" {
                    return Err(syn::Error::new_spanned(
                        &expr,
                        "only `..Default::default()` is supported here",
                    ));
                }
                if !ov_content.is_empty() {
                    return Err(ov_content.error("`..Default::default()` must come last"));
                }
                defaults = true;
                break;
            }
            let field: Ident = ov_content.parse()?;
            if ov_content.peek(Token![=>]) {
                // Enum variant rename: `SrcVariant => DestVariant`.
                ov_content.parse::<Token![=>]>()?;
                let dest_variant: Ident = ov_content.parse()?;
                variant_renames.push((field, dest_variant));
            } else {
                ov_content.parse::<Token![:]>()?;
                let expr: syn::Expr = ov_content.parse()?;
                overrides.push((field, expr));
            }
            if !ov_content.is_empty() {
                ov_content.parse::<Token![,]>()?;
            }
        }

        Ok(Self {
            collected,
            dest_shape,
            func,
            src,
            dest,
            overrides,
            lets,
            variant_renames,
            defaults,
        })
    }
}

/// A tuple element is OPEN (its fields join the auto-match, so it must derive
/// `MagicMap`) when it is a plain non-generic type path that isn't a known
/// leaf. Leaves, generic types (`Option<_>`, `DateTime<Utc>`), references,
/// nested tuples, … are OPAQUE: reachable only as `src.N` in overrides.
fn open_element(ty: &syn::Type) -> Option<&Path> {
    let syn::Type::Path(tp) = ty else { return None };
    let path = &tp.path;
    if tp.qself.is_some() || path.segments.iter().any(|s| !s.arguments.is_empty()) {
        return None;
    }
    let leaf = matches!(
        path.segments.last().unwrap().ident.to_string().as_str(),
        "bool"
            | "char"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "String"
            | "Uuid"
            | "Decimal"
    );
    (!leaf).then_some(path)
}

/// Where a non-overridden destination field comes from.
enum FieldSource {
    Direct,         // single-path src: `src.field`, compiler-checked
    Element(usize), // tuple src: `src.N.field`
}

pub fn expand(raw: TokenStream2, input: ExpandInput) -> Result<TokenStream, syn::Error> {
    let ExpandInput {
        collected,
        dest_shape,
        func,
        src,
        dest,
        overrides,
        lets,
        variant_renames,
        defaults,
    } = input;

    let tuple_elems: Option<Vec<&syn::Type>> = match &src {
        syn::Type::Tuple(t) => Some(t.elems.iter().collect()),
        _ => None,
    };

    // Fixpoint: chain into the schema macro of the first open tuple element
    // whose shape hasn't been collected yet, re-feeding the raw input. When
    // overrides already cover every destination field, no schema is needed at
    // all — this lets schema-less foreign types (errors, service structs) ride
    // along in tuples as long as they're consumed explicitly.
    let all_overridden = !dest_shape.is_enum
        && dest_shape
            .names
            .iter()
            .all(|f| overrides.iter().any(|(name, _)| name == f));
    if !all_overridden {
        if let Some(elems) = &tuple_elems {
            for (idx, ty) in elems.iter().enumerate() {
                let Some(path) = open_element(ty) else {
                    continue;
                };
                if collected.iter().any(|(i, _)| *i == idx) {
                    continue;
                }
                let mut schema = path.clone();
                let last = schema.segments.last_mut().unwrap();
                last.ident = format_ident!("__magic_map_schema_{}", last.ident);
                let idx_lit = syn::Index::from(idx);
                return Ok(quote! { #schema! { @for(#idx_lit) #raw } }.into());
            }
        } else if (defaults || dest_shape.is_enum) && collected.is_empty() {
            // `..Default::default()` on a single-path source: knowing which
            // dest fields the source CAN provide requires its schema too.
            // Enum destinations need the source's variant list for the same
            // reason — arms are generated per SOURCE variant.
            let Some(path) = open_element(&src) else {
                return Err(syn::Error::new_spanned(
                    &src,
                    "this mapping needs a plain MagicMap-derived source type",
                ));
            };
            let mut schema = path.clone();
            let last = schema.segments.last_mut().unwrap();
            last.ident = format_ident!("__magic_map_schema_{}", last.ident);
            return Ok(quote! { #schema! { @for(0) #raw } }.into());
        }
    }

    // Hygiene: by the time this proc macro runs, Span::call_site() is the
    // schema macro_rules' expansion context — a `src` binder created there is
    // invisible to the user's `src` inside override expressions (E0425). Forge
    // the binder from a user-written token's span so they share context.
    let src_var = Ident::new("src", syn::spanned::Spanned::span(&src));

    let body = if dest_shape.is_enum {
        if tuple_elems.is_some() {
            return Err(syn::Error::new_spanned(
                &src,
                "enum destinations take a single source type, not a tuple",
            ));
        }
        if let Some((field, _)) = overrides.first() {
            return Err(syn::Error::new(
                field.span(),
                "enum mappings take `SrcVariant => DestVariant` renames, not field overrides",
            ));
        }
        if let Some(stmt) = lets.first() {
            return Err(syn::Error::new_spanned(
                stmt,
                "`let` bindings only apply to struct destinations",
            ));
        }
        let Some((_, src_shape)) = collected.first() else {
            return Err(syn::Error::new_spanned(
                &src,
                "enum mapping needs the source's MagicMap schema",
            ));
        };
        if !src_shape.is_enum {
            return Err(syn::Error::new_spanned(
                &src,
                "enum destinations need an enum source",
            ));
        }
        // Variant-by-name, generated per SOURCE variant so extra destination
        // variants are simply never produced. Explicit `Src => Dest` renames
        // win; then same-name; the proto3 zero variant `Unspecified` folds to
        // the destination's declared default; anything else is a compile
        // error.
        if let Some((bad, _)) = variant_renames
            .iter()
            .find(|(v, _)| !src_shape.names.contains(v))
        {
            return Err(syn::Error::new(
                bad.span(),
                format!("`{bad}` is not a variant of the source enum"),
            ));
        }
        if let Some((_, bad)) = variant_renames
            .iter()
            .find(|(_, d)| !dest_shape.names.contains(d))
        {
            return Err(syn::Error::new(
                bad.span(),
                format!("`{bad}` is not a variant of the destination enum"),
            ));
        }
        let mut arms = Vec::new();
        for v in &src_shape.names {
            if let Some((_, d)) = variant_renames.iter().find(|(sv, _)| sv == v) {
                arms.push(quote! { #src::#v => #dest::#d });
            } else if dest_shape.names.contains(v) {
                arms.push(quote! { #src::#v => #dest::#v });
            } else if v == "Unspecified" {
                arms.push(quote! {
                    #src::#v => <#dest as ::core::default::Default>::default()
                });
            } else {
                return Err(syn::Error::new_spanned(
                    &src,
                    format!("source variant `{v}` has no same-named destination variant"),
                ));
            }
        }
        quote! { ::core::result::Result::Ok(match #src_var { #(#arms),* }) }
    } else {
        if let Some((bad, _)) = variant_renames.first() {
            return Err(syn::Error::new(
                bad.span(),
                "`Src => Dest` variant renames only apply to enum destinations",
            ));
        }
        // An override naming a non-existent dest field would otherwise vanish
        // silently — reject it here.
        if let Some((bad, _)) = overrides
            .iter()
            .find(|(name, _)| !dest_shape.names.contains(name))
        {
            return Err(syn::Error::new(
                bad.span(),
                format!("`{bad}` is not a field of the destination type"),
            ));
        }

        let mut assigns = Vec::new();
        let mut defaulted = false;
        for f in &dest_shape.names {
            if let Some((_, expr)) = overrides.iter().find(|(name, _)| name == f) {
                assigns.push(quote! { #f: #expr });
                continue;
            }
            let source = if tuple_elems.is_none() {
                if defaults {
                    // Schema collected above — absent fields fall to Default.
                    let present = collected
                        .iter()
                        .any(|(_, s)| !s.is_enum && s.names.contains(f));
                    if !present {
                        defaulted = true;
                        continue;
                    }
                }
                FieldSource::Direct
            } else {
                // Auto-match across open struct elements (enum shapes have
                // variants, not fields — they never contribute).
                let hits: Vec<usize> = collected
                    .iter()
                    .filter(|(_, s)| !s.is_enum && s.names.contains(f))
                    .map(|(i, _)| *i)
                    .collect();
                match hits.as_slice() {
                    [i] => FieldSource::Element(*i),
                    [] if defaults => {
                        defaulted = true;
                        continue;
                    }
                    [] => {
                        return Err(syn::Error::new(
                            f.span(),
                            format!(
                                "`{f}` is not a field of any tuple element — \
                                 add an override (`{f}: src.N...`)"
                            ),
                        ))
                    }
                    many => {
                        return Err(syn::Error::new(
                            f.span(),
                            format!(
                                "`{f}` is ambiguous — found in tuple elements {many:?}; \
                                 add an explicit override (`{f}: src.{}.{f}`)",
                                many[0]
                            ),
                        ))
                    }
                }
            };
            let access = match source {
                FieldSource::Direct => quote! { #src_var.#f },
                FieldSource::Element(i) => {
                    let i = syn::Index::from(i);
                    quote! { #src_var.#i.#f }
                }
            };
            if defaults {
                // Autoref-specialized, three tiers: `Option<S>` sources funnel
                // their inner value (None → default instance's field); plain
                // funnel next (Option→Option stays None→None); plain source
                // into an `Option` dest wraps in `Some`.
                assigns.push(quote! { #f: {
                    use ::magic_map::{
                        MapFieldOpt as _, MapFieldVal as _, MapFieldWrap as _,
                    };
                    (&mut &mut &mut ::magic_map::MapPair(
                        ::core::option::Option::Some(#access),
                        ::core::option::Option::Some(__magic_fb.#f),
                    ))
                        .map_field_or()?
                } });
            } else {
                assigns.push(quote! { #f: ::magic_map::MapFrom::map_from(#access)? });
            }
        }
        let prelude = defaults.then(|| {
            quote! { let __magic_fb = <#dest as ::core::default::Default>::default(); }
        });
        let trailer = defaulted.then(|| quote! { ..::core::default::Default::default() });
        quote! { #prelude #(#lets)* ::core::result::Result::Ok(#dest { #(#assigns,)* #trailer }) }
    };

    Ok(match func {
        Some((vis, name)) => quote! {
            #vis fn #name(
                #src_var: #src,
            ) -> ::core::result::Result<#dest, ::magic_map::MappingError> {
                #body
            }
        },
        None => quote! {
            impl ::magic_map::MapFrom<#src> for #dest {
                fn map_from(
                    #src_var: #src,
                ) -> ::core::result::Result<Self, ::magic_map::MappingError> {
                    #body
                }
            }
        },
    }
    .into())
}
