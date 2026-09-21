//! Derive macros for dcbor `CBOREncodable` and `CBORDecodable` traits.
//!
//! Generates `From<T> for CBOR` and `TryFrom<CBOR> for T` implementations
//! from struct and enum definitions.
//!
//! # Struct encoding
//!
//! Structs are encoded as CBOR maps with integer keys for compactness.
//! Each field must have a `#[cbor(n = N)]` attribute specifying its key.
//! Fields of type `Option<T>` are omitted from the map when `None`.
//!
//! ```ignore
//! use dcbor::prelude::*;
//! use dcbor_derive::{CBOREncodable, CBORDecodable};
//!
//! #[derive(Clone, CBOREncodable, CBORDecodable)]
//! struct Person {
//!     #[cbor(n = 0)]
//!     name: String,
//!     #[cbor(n = 1)]
//!     age: u8,
//!     #[cbor(n = 2)]
//!     email: Option<String>,
//! }
//! ```
//!
//! # Newtype structs
//!
//! Single-field tuple structs delegate to the inner type:
//!
//! ```ignore
//! #[derive(Clone, CBOREncodable, CBORDecodable)]
//! struct Wrapper(String);
//! ```
//!
//! # Enums
//!
//! Enums encode as CBOR arrays `[variant_index, ...fields]`.
//! Unit variants encode as a bare integer.
//! Tuple variants encode as `[variant_index, field0, field1, ...]`.
//! Struct variants encode as `[variant_index, {map of fields}]`.
//!
//! Use `#[cbor(n = N)]` on each variant to set the discriminant.
//!
//! ```ignore
//! #[derive(Clone, CBOREncodable, CBORDecodable)]
//! enum Shape {
//!     #[cbor(n = 0)]
//!     Circle { #[cbor(n = 0)] radius: f64 },
//!     #[cbor(n = 1)]
//!     Rectangle {
//!         #[cbor(n = 0)] width: f64,
//!         #[cbor(n = 1)] height: f64,
//!     },
//!     #[cbor(n = 2)]
//!     Point,
//! }
//! ```
//!
//! # Tagged types
//!
//! Use `#[cbor(tag = N)]` on the type to generate `CBORTagged`,
//! `CBORTaggedEncodable`, and `CBORTaggedDecodable` implementations.
//! The `TryFrom<CBOR>` impl will expect and validate the tag.
//!
//! ```ignore
//! #[derive(Clone, CBOREncodable, CBORDecodable)]
//! #[cbor(tag = 42)]
//! struct Tagged {
//!     #[cbor(n = 0)]
//!     data: Vec<u8>,
//! }
//! ```

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Data, DataEnum, DataStruct, DeriveInput, Error, Fields, Ident, Result,
    parse_macro_input,
};

/// Derive `From<T> for dcbor::CBOR`, making the type CBOREncodable.
#[proc_macro_derive(CBOREncodable, attributes(cbor))]
pub fn derive_cbor_encodable(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_encodable(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Derive `TryFrom<dcbor::CBOR> for T`, making the type CBORDecodable.
#[proc_macro_derive(CBORDecodable, attributes(cbor))]
pub fn derive_cbor_decodable(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_decodable(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Derive both `From<T> for CBOR` and `TryFrom<CBOR> for T`.
#[proc_macro_derive(CBORCodable, attributes(cbor))]
pub fn derive_cbor_codable(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let enc = expand_encodable(&input);
    let dec = expand_decodable(&input);
    match (enc, dec) {
        (Ok(e), Ok(d)) => quote! { #e #d }.into(),
        (Err(e), _) => e.to_compile_error().into(),
        (_, Err(e)) => e.to_compile_error().into(),
    }
}

// ---------------------------------------------------------------------------
// Attribute parsing
// ---------------------------------------------------------------------------

struct FieldAttr {
    key: u64,
}

struct TypeAttr {
    tag: Option<u64>,
}

struct VariantAttr {
    discriminant: u64,
}

fn parse_field_attr(field: &syn::Field) -> Result<FieldAttr> {
    for attr in &field.attrs {
        if attr.path().is_ident("cbor") {
            let mut key = None;
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("n") {
                    let value = meta.value()?;
                    let lit: syn::LitInt = value.parse()?;
                    key = Some(lit.base10_parse::<u64>()?);
                    Ok(())
                } else {
                    Err(meta.error("expected `n = N`"))
                }
            })?;
            match key {
                Some(k) => return Ok(FieldAttr { key: k }),
                None => {
                    return Err(Error::new_spanned(attr, "missing `n = N` in #[cbor(...)]"))
                }
            }
        }
    }
    Err(Error::new_spanned(
        field,
        "missing #[cbor(n = N)] attribute",
    ))
}

fn parse_type_attr(input: &DeriveInput) -> Result<TypeAttr> {
    let mut tag = None;
    for attr in &input.attrs {
        if attr.path().is_ident("cbor") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("tag") {
                    let value = meta.value()?;
                    let lit: syn::LitInt = value.parse()?;
                    tag = Some(lit.base10_parse::<u64>()?);
                    Ok(())
                } else {
                    Err(meta.error("expected `tag = N`"))
                }
            })?;
        }
    }
    Ok(TypeAttr { tag })
}

fn parse_variant_attr(variant: &syn::Variant) -> Result<VariantAttr> {
    for attr in &variant.attrs {
        if attr.path().is_ident("cbor") {
            let mut disc = None;
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("n") {
                    let value = meta.value()?;
                    let lit: syn::LitInt = value.parse()?;
                    disc = Some(lit.base10_parse::<u64>()?);
                    Ok(())
                } else {
                    Err(meta.error("expected `n = N`"))
                }
            })?;
            match disc {
                Some(d) => return Ok(VariantAttr { discriminant: d }),
                None => {
                    return Err(Error::new_spanned(attr, "missing `n = N` in #[cbor(...)]"))
                }
            }
        }
    }
    Err(Error::new_spanned(
        variant,
        "missing #[cbor(n = N)] attribute on variant",
    ))
}

fn is_option(ty: &syn::Type) -> bool {
    if let syn::Type::Path(p) = ty {
        if let Some(seg) = p.path.segments.last() {
            return seg.ident == "Option";
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Encodable expansion
// ---------------------------------------------------------------------------

fn expand_encodable(input: &DeriveInput) -> Result<TokenStream2> {
    let type_attr = parse_type_attr(input)?;
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let core_from = match &input.data {
        Data::Struct(data) => expand_encodable_struct(name, data)?,
        Data::Enum(data) => expand_encodable_enum(name, data)?,
        Data::Union(_) => {
            return Err(Error::new_spanned(input, "unions are not supported"))
        }
    };

    let tagged_impls = if let Some(tag_value) = type_attr.tag {
        expand_tagged_encodable(name, &impl_generics, &ty_generics, where_clause, tag_value)
    } else {
        quote! {}
    };

    Ok(quote! {
        impl #impl_generics From<#name #ty_generics> for dcbor::CBOR #where_clause {
            fn from(value: #name #ty_generics) -> Self {
                #core_from
            }
        }
        #tagged_impls
    })
}

fn expand_encodable_struct(name: &Ident, data: &DataStruct) -> Result<TokenStream2> {
    match &data.fields {
        Fields::Named(fields) => {
            let mut inserts = Vec::new();
            for field in &fields.named {
                let attr = parse_field_attr(field)?;
                let field_name = field.ident.as_ref().unwrap();
                let key = attr.key;
                if is_option(&field.ty) {
                    inserts.push(quote! {
                        if let Some(ref v) = value.#field_name {
                            map.insert(#key, v.clone());
                        }
                    });
                } else {
                    inserts.push(quote! {
                        map.insert(#key, value.#field_name.clone());
                    });
                }
            }
            Ok(quote! {
                let mut map = dcbor::Map::new();
                #(#inserts)*
                map.into()
            })
        }
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
            Ok(quote! {
                value.0.into()
            })
        }
        Fields::Unnamed(_) => {
            Err(Error::new_spanned(
                name,
                "tuple structs with more than one field are not supported; \
                 use a named struct with #[cbor(n = N)] attributes",
            ))
        }
        Fields::Unit => Ok(quote! { dcbor::CBOR::null() }),
    }
}

fn expand_encodable_enum(name: &Ident, data: &DataEnum) -> Result<TokenStream2> {
    let mut arms = Vec::new();
    for variant in &data.variants {
        let vattr = parse_variant_attr(variant)?;
        let disc = vattr.discriminant;
        let vname = &variant.ident;
        match &variant.fields {
            Fields::Unit => {
                arms.push(quote! {
                    #name::#vname => dcbor::CBOR::from(#disc),
                });
            }
            Fields::Unnamed(fields) => {
                let bindings: Vec<Ident> = (0..fields.unnamed.len())
                    .map(|i| Ident::new(&format!("f{}", i), proc_macro2::Span::call_site()))
                    .collect();
                let pattern = quote! { #name::#vname(#(#bindings),*) };
                let elements: Vec<TokenStream2> = std::iter::once(
                    quote! { dcbor::CBOR::from(#disc) }
                ).chain(
                    bindings.iter().map(|b| quote! { #b.clone().into() })
                ).collect();
                arms.push(quote! {
                    #pattern => {
                        let arr: Vec<dcbor::CBOR> = vec![#(#elements),*];
                        arr.into()
                    }
                });
            }
            Fields::Named(fields) => {
                let field_names: Vec<&Ident> = fields.named.iter()
                    .map(|f| f.ident.as_ref().unwrap())
                    .collect();
                let pattern = quote! { #name::#vname { #(#field_names),* } };
                let mut inserts = Vec::new();
                for field in &fields.named {
                    let attr = parse_field_attr(field)?;
                    let fname = field.ident.as_ref().unwrap();
                    let key = attr.key;
                    if is_option(&field.ty) {
                        inserts.push(quote! {
                            if let Some(ref v) = #fname {
                                inner_map.insert(#key, v.clone());
                            }
                        });
                    } else {
                        inserts.push(quote! {
                            inner_map.insert(#key, #fname.clone());
                        });
                    }
                }
                arms.push(quote! {
                    #pattern => {
                        let mut inner_map = dcbor::Map::new();
                        #(#inserts)*
                        let arr: Vec<dcbor::CBOR> = vec![
                            dcbor::CBOR::from(#disc),
                            inner_map.into(),
                        ];
                        arr.into()
                    }
                });
            }
        }
    }
    Ok(quote! {
        match value {
            #(#arms)*
        }
    })
}

fn expand_tagged_encodable(
    name: &Ident,
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: Option<&syn::WhereClause>,
    tag_value: u64,
) -> TokenStream2 {
    quote! {
        impl #impl_generics dcbor::CBORTagged for #name #ty_generics #where_clause {
            fn cbor_tags() -> Vec<dcbor::Tag> {
                vec![dcbor::Tag::with_value(#tag_value)]
            }
        }

        impl #impl_generics dcbor::CBORTaggedEncodable for #name #ty_generics #where_clause {
            fn untagged_cbor(&self) -> dcbor::CBOR {
                self.clone().into()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Decodable expansion
// ---------------------------------------------------------------------------

fn expand_decodable(input: &DeriveInput) -> Result<TokenStream2> {
    let type_attr = parse_type_attr(input)?;
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let core_decode = match &input.data {
        Data::Struct(data) => expand_decodable_struct(name, data)?,
        Data::Enum(data) => expand_decodable_enum(name, data)?,
        Data::Union(_) => {
            return Err(Error::new_spanned(input, "unions are not supported"))
        }
    };

    let tagged_impls = if let Some(tag_value) = type_attr.tag {
        let tagged_decode = expand_tagged_decodable(
            name, &impl_generics, &ty_generics, where_clause, tag_value, &core_decode,
        );
        // For tagged types, TryFrom<CBOR> expects a tagged wrapper
        quote! {
            #tagged_decode

            impl #impl_generics TryFrom<dcbor::CBOR> for #name #ty_generics #where_clause {
                type Error = dcbor::Error;

                fn try_from(cbor: dcbor::CBOR) -> dcbor::Result<Self> {
                    Self::from_tagged_cbor(cbor)
                }
            }
        }
    } else {
        // For untagged types, TryFrom<CBOR> decodes directly
        quote! {
            impl #impl_generics TryFrom<dcbor::CBOR> for #name #ty_generics #where_clause {
                type Error = dcbor::Error;

                fn try_from(cbor: dcbor::CBOR) -> dcbor::Result<Self> {
                    #core_decode
                }
            }
        }
    };

    Ok(tagged_impls)
}

fn expand_decodable_struct(name: &Ident, data: &DataStruct) -> Result<TokenStream2> {
    match &data.fields {
        Fields::Named(fields) => {
            let mut extracts = Vec::new();
            let mut field_names = Vec::new();
            for field in &fields.named {
                let attr = parse_field_attr(field)?;
                let fname = field.ident.as_ref().unwrap();
                let key = attr.key;
                field_names.push(fname);
                if is_option(&field.ty) {
                    extracts.push(quote! {
                        let #fname = map.get(#key);
                    });
                } else {
                    extracts.push(quote! {
                        let #fname = map.extract(#key)?;
                    });
                }
            }
            Ok(quote! {
                match cbor.into_case() {
                    dcbor::CBORCase::Map(map) => {
                        #(#extracts)*
                        Ok(#name { #(#field_names),* })
                    }
                    _ => Err(dcbor::Error::WrongType),
                }
            })
        }
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
            Ok(quote! {
                Ok(#name(cbor.try_into()?))
            })
        }
        Fields::Unnamed(_) => {
            Err(Error::new_spanned(
                name,
                "tuple structs with more than one field are not supported; \
                 use a named struct with #[cbor(n = N)] attributes",
            ))
        }
        Fields::Unit => {
            Ok(quote! {
                if cbor.is_null() {
                    Ok(#name)
                } else {
                    Err(dcbor::Error::WrongType)
                }
            })
        }
    }
}

fn expand_decodable_enum(name: &Ident, data: &DataEnum) -> Result<TokenStream2> {
    let mut unit_arms = Vec::new();
    let mut array_arms = Vec::new();

    for variant in &data.variants {
        let vattr = parse_variant_attr(variant)?;
        let disc = vattr.discriminant;
        let vname = &variant.ident;

        match &variant.fields {
            Fields::Unit => {
                unit_arms.push(quote! {
                    #disc => Ok(#name::#vname),
                });
            }
            Fields::Unnamed(fields) => {
                let field_count = fields.unnamed.len();
                let field_extracts: Vec<TokenStream2> = (0..field_count)
                    .map(|i| {
                        let idx = i + 1; // skip discriminant at index 0
                        quote! {
                            arr[#idx].clone().try_into()?
                        }
                    })
                    .collect();
                let expected_len = field_count + 1;
                array_arms.push(quote! {
                    #disc => {
                        if arr.len() != #expected_len {
                            return Err(dcbor::Error::WrongType);
                        }
                        Ok(#name::#vname(#(#field_extracts),*))
                    }
                });
            }
            Fields::Named(fields) => {
                let mut extracts = Vec::new();
                let mut field_names = Vec::new();
                for field in &fields.named {
                    let attr = parse_field_attr(field)?;
                    let fname = field.ident.as_ref().unwrap();
                    let key = attr.key;
                    field_names.push(fname);
                    if is_option(&field.ty) {
                        extracts.push(quote! {
                            let #fname = inner_map.get(#key);
                        });
                    } else {
                        extracts.push(quote! {
                            let #fname = inner_map.extract(#key)?;
                        });
                    }
                }
                array_arms.push(quote! {
                    #disc => {
                        if arr.len() != 2 {
                            return Err(dcbor::Error::WrongType);
                        }
                        match arr[1].clone().into_case() {
                            dcbor::CBORCase::Map(inner_map) => {
                                #(#extracts)*
                                Ok(#name::#vname { #(#field_names),* })
                            }
                            _ => Err(dcbor::Error::WrongType),
                        }
                    }
                });
            }
        }
    }

    // If all variants are unit, decode from bare integer
    // If mixed, decode from integer (unit) or array (non-unit)
    let has_non_unit = !array_arms.is_empty();
    let has_unit = !unit_arms.is_empty();

    if has_non_unit && has_unit {
        Ok(quote! {
            match cbor.clone().into_case() {
                dcbor::CBORCase::Unsigned(disc) => {
                    match disc {
                        #(#unit_arms)*
                        _ => Err(dcbor::Error::WrongType),
                    }
                }
                dcbor::CBORCase::Array(arr) => {
                    if arr.is_empty() {
                        return Err(dcbor::Error::WrongType);
                    }
                    let disc: u64 = arr[0].clone().try_into()?;
                    match disc {
                        #(#array_arms)*
                        _ => Err(dcbor::Error::WrongType),
                    }
                }
                _ => Err(dcbor::Error::WrongType),
            }
        })
    } else if has_non_unit {
        Ok(quote! {
            match cbor.into_case() {
                dcbor::CBORCase::Array(arr) => {
                    if arr.is_empty() {
                        return Err(dcbor::Error::WrongType);
                    }
                    let disc: u64 = arr[0].clone().try_into()?;
                    match disc {
                        #(#array_arms)*
                        _ => Err(dcbor::Error::WrongType),
                    }
                }
                _ => Err(dcbor::Error::WrongType),
            }
        })
    } else {
        Ok(quote! {
            match cbor.into_case() {
                dcbor::CBORCase::Unsigned(disc) => {
                    match disc {
                        #(#unit_arms)*
                        _ => Err(dcbor::Error::WrongType),
                    }
                }
                _ => Err(dcbor::Error::WrongType),
            }
        })
    }
}

fn expand_tagged_decodable(
    name: &Ident,
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: Option<&syn::WhereClause>,
    _tag_value: u64,
    core_decode: &TokenStream2,
) -> TokenStream2 {
    let _ = _tag_value; // tag value already in CBORTagged impl from encodable
    quote! {
        impl #impl_generics dcbor::CBORTaggedDecodable for #name #ty_generics #where_clause {
            fn from_untagged_cbor(cbor: dcbor::CBOR) -> dcbor::Result<Self> {
                #core_decode
            }
        }
    }
}
