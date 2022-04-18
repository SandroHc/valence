// TODO: can't match on str in const fn.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::{env, fs};

use anyhow::Context;
use heck::{ToPascalCase, ToShoutySnakeCase};
use proc_macro2::TokenStream;
use quote::quote;
use serde::Deserialize;

use crate::ident;

pub fn build() -> anyhow::Result<()> {
    let blocks = parse_blocks_json()?;

    let max_block_state = blocks.iter().map(|b| b.max_state_id).max().unwrap();

    let state_to_type = blocks
        .iter()
        .map(|b| {
            let min = b.min_state_id;
            let max = b.max_state_id;
            let name = ident(b.name.to_pascal_case());
            quote! {
                #min..=#max => BlockType::#name,
            }
        })
        .collect::<TokenStream>();

    let get_arms = blocks
        .iter()
        .filter(|&b| !b.props.is_empty())
        .map(|b| {
            let block_type_name = ident(b.name.to_pascal_case());

            let arms = b
                .props
                .iter()
                .map(|p| {
                    let prop_name = ident(p.name.to_pascal_case());
                    let min_state_id = b.min_state_id;
                    let product: u16 = b
                        .props
                        .iter()
                        .take_while(|&other| p.name != other.name)
                        .map(|p| p.vals.len() as u16)
                        .product();

                    let num_values = p.vals.len() as u16;

                    let arms = p.vals.iter().enumerate().map(|(i, v)| {
                        let value_idx = i as u16;
                        let value_name = ident(v.to_pascal_case());
                        quote! {
                            #value_idx => Some(PropValue::#value_name),
                        }
                    }).collect::<TokenStream>();

                    quote! {
                        PropName::#prop_name => match (self.0 - #min_state_id) / #product % #num_values {
                            #arms
                            _ => unreachable!(),
                        },
                    }
                })
                .collect::<TokenStream>();

            quote! {
                BlockType::#block_type_name => match name {
                    #arms
                    _ => None,
                },
            }
        })
        .collect::<TokenStream>();

    let set_arms = blocks
        .iter()
        .filter(|&b| !b.props.is_empty())
        .map(|b| {
            let block_type_name = ident(b.name.to_pascal_case());

            let arms = b
                .props
                .iter()
                .map(|p| {
                    let prop_name = ident(p.name.to_pascal_case());
                    let min_state_id = b.min_state_id;
                    let product: u16 = b
                        .props
                        .iter()
                        .take_while(|&other| p.name != other.name)
                        .map(|p| p.vals.len() as u16)
                        .product();

                    let num_values = p.vals.len() as u16;

                    let arms = p
                        .vals
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let val_idx = i as u16;
                            let val_name = ident(v.to_pascal_case());
                            quote! {
                                PropValue::#val_name =>
                                    Self(self.0 - (self.0 - #min_state_id) / #product % #num_values * #product
                                        + #val_idx * #product),
                            }
                        })
                        .collect::<TokenStream>();

                    quote! {
                        PropName::#prop_name => match val {
                            #arms
                            _ => self,
                        },
                    }
                })
                .collect::<TokenStream>();

            quote! {
                BlockType::#block_type_name => match name {
                    #arms
                    _ => self,
                },
            }
        })
        .collect::<TokenStream>();

    let default_block_states = blocks
        .iter()
        .map(|b| {
            let name = ident(b.name.to_shouty_snake_case());
            let state = b.default_state;
            let doc = format!("The default block state for `{}`.", b.name);
            quote! {
                #[doc = #doc]
                pub const #name: BlockState = BlockState(#state);
            }
        })
        .collect::<TokenStream>();

    let type_to_state = blocks
        .iter()
        .map(|b| {
            let typ = ident(b.name.to_pascal_case());
            let state = ident(b.name.to_shouty_snake_case());
            quote! {
                BlockType::#typ => BlockState::#state,
            }
        })
        .collect::<TokenStream>();

    let block_type_variants = blocks
        .iter()
        .map(|b| ident(b.name.to_pascal_case()))
        .collect::<Vec<_>>();

    let block_type_from_str_arms = blocks
        .iter()
        .map(|b| {
            let name = &b.name;
            let name_ident = ident(name.to_pascal_case());
            quote! {
                #name => Some(BlockType::#name_ident),
            }
        })
        .collect::<TokenStream>();

    let block_type_to_str_arms = blocks
        .iter()
        .map(|b| {
            let name = &b.name;
            let name_ident = ident(name.to_pascal_case());
            quote! {
                BlockType::#name_ident => #name,
            }
        })
        .collect::<TokenStream>();

    let block_type_props_arms = blocks
        .iter()
        .filter(|&b| !b.props.is_empty())
        .map(|b| {
            let name = ident(b.name.to_pascal_case());
            let prop_names = b.props.iter().map(|p| ident(p.name.to_pascal_case()));

            quote! {
                Self::#name => &[#(PropName::#prop_names,)*],
            }
        })
        .collect::<TokenStream>();

    let num_block_types = blocks.len();

    let prop_names = blocks
        .iter()
        .flat_map(|b| b.props.iter().map(|p| &p.name))
        .collect::<BTreeSet<_>>();

    let prop_name_variants = prop_names
        .iter()
        .map(|&name| ident(name.to_pascal_case()))
        .collect::<Vec<_>>();

    let prop_name_from_str_arms = prop_names
        .iter()
        .map(|&name| {
            let ident = ident(name.to_pascal_case());
            quote! {
                #name => Some(PropName::#ident),
            }
        })
        .collect::<TokenStream>();

    let prop_name_to_str_arms = prop_names
        .iter()
        .map(|&name| {
            let ident = ident(name.to_pascal_case());
            quote! {
                PropName::#ident => #name,
            }
        })
        .collect::<TokenStream>();

    let num_prop_names = prop_names.len();

    let prop_values = blocks
        .iter()
        .flat_map(|b| b.props.iter().flat_map(|p| &p.vals))
        .collect::<BTreeSet<_>>();

    let prop_value_variants = prop_values
        .iter()
        .map(|val| ident(val.to_pascal_case()))
        .collect::<Vec<_>>();

    let prop_value_from_str_arms = prop_values
        .iter()
        .map(|val| {
            let ident = ident(val.to_pascal_case());
            quote! {
                #val => Some(PropValue::#ident),
            }
        })
        .collect::<TokenStream>();

    let prop_value_to_str_arms = prop_values
        .iter()
        .map(|val| {
            let ident = ident(val.to_pascal_case());
            quote! {
                PropValue::#ident => #val,
            }
        })
        .collect::<TokenStream>();

    let prop_value_from_u16_arms = prop_values
        .iter()
        .filter_map(|v| v.parse::<u16>().ok())
        .map(|n| {
            let ident = ident(n.to_string());
            quote! {
                #n => Some(PropValue::#ident),
            }
        })
        .collect::<TokenStream>();

    let prop_value_to_u16_arms = prop_values
        .iter()
        .filter_map(|v| v.parse::<u16>().ok())
        .map(|n| {
            let ident = ident(n.to_string());
            quote! {
                PropValue::#ident => Some(#n),
            }
        })
        .collect::<TokenStream>();

    let num_property_names = prop_values.len();

    let finshed = quote! {
        /// Represents the state of a block, not including block entity data such as
        /// the text on a sign, the design on a banner, or the content of a spawner.
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
        pub struct BlockState(u16);

        impl BlockState {
            /// Returns the default block state for a given block type.
            pub const fn from_type(typ: BlockType) -> Self {
                match typ {
                    #type_to_state
                }
            }

            /// Returns the [`BlockType`] of this block state.
            pub const fn to_type(self) -> BlockType {
                match self.0 {
                    #state_to_type
                    _ => unreachable!(),
                }
            }

            /// Constructs a block state from a raw block state ID.
            ///
            /// If the given ID is invalid, `None` is returned.
            pub const fn from_raw(id: u16) -> Option<Self> {
                if id <= #max_block_state {
                    Some(Self(id))
                } else {
                    None
                }
            }

            /// Converts this block state to its underlying raw block state ID.
            ///
            /// The original block state can be recovered with [`BlockState::from_raw`].
            pub const fn to_raw(self) -> u16 {
                self.0
            }

            /// Gets the value of the property with the given name from this block.
            ///
            /// If this block does not have the property, then `None` is returned.
            pub const fn get(self, name: PropName) -> Option<PropValue> {
                match self.to_type() {
                    #get_arms
                    _ => None
                }
            }

            /// Sets the value of a propery on this block, returning the modified block.
            ///
            /// If this block does not have the given property or the property value is invalid,
            /// then the orginal block is returned unchanged.
            #[must_use]
            pub const fn set(self, name: PropName, val: PropValue) -> Self {
                match self.to_type() {
                    #set_arms
                    _ => self,
                }
            }

            #default_block_states
        }

        /// An enumeration of all block types.
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub enum BlockType {
            #(#block_type_variants,)*
        }

        impl BlockType {
            /// Construct a block type from its snake_case name.
            ///
            /// Returns `None` if the given name is not valid.
            pub fn from_str(name: &str) -> Option<BlockType> {
                match name {
                    #block_type_from_str_arms
                    _ => None
                }
            }

            /// Get the snake_case name of this block type.
            pub const fn to_str(self) -> &'static str {
                match self {
                    #block_type_to_str_arms
                }
            }

            /// Returns the default block state for a given block type.
            pub const fn to_state(self) -> BlockState {
                BlockState::from_type(self)
            }

            /// Returns a slice of all properties this block type has.
            pub const fn props(self) -> &'static [PropName] {
                match self {
                    #block_type_props_arms
                    _ => &[],
                }
            }

            /// An array of all block types.
            pub const ALL: [Self; #num_block_types] = [#(Self::#block_type_variants,)*];
        }

        /// The default block type is `air`.
        impl Default for BlockType {
            fn default() -> Self {
                Self::Air
            }
        }

        /// Contains all possible block state property names.
        ///
        /// For example, `waterlogged`, `facing`, and `half` are all property names.
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub enum PropName {
            #(#prop_name_variants,)*
        }

        impl PropName {
            /// Construct a property name from its snake_case name.
            ///
            /// Returns `None` if the given name is not valid.
            pub fn from_str(name: &str) -> Option<Self> {
                match name {
                    #prop_name_from_str_arms
                    _ => None,
                }
            }

            /// Get the snake_case name of this property name.
            pub const fn to_str(self) -> &'static str {
                match self {
                    #prop_name_to_str_arms
                }
            }

            /// An array of all property names.
            pub const ALL: [Self; #num_prop_names] = [#(Self::#prop_name_variants,)*];
        }

        /// Contains all possible values that a block property might have.
        ///
        /// For example, `upper`, `true`, and `2` are all property values.
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub enum PropValue {
            #(#prop_value_variants,)*
        }

        impl PropValue {
            /// Construct a property value from its snake_case name.
            ///
            /// Returns `None` if the given name is not valid.
            pub fn from_str(name: &str) -> Option<Self> {
                match name {
                    #prop_value_from_str_arms
                    _ => None,
                }
            }

            /// Get the snake_case name of this property value.
            pub const fn to_str(self) -> &'static str {
                match self {
                    #prop_value_to_str_arms
                }
            }

            /// Converts a `u16` into a numeric property value.
            /// Returns `None` if the given number does not have a
            /// corresponding property value.
            pub const fn from_u16(n: u16) -> Option<Self> {
                match n {
                    #prop_value_from_u16_arms
                    _ => None,
                }
            }

            /// Converts this property value into a `u16` if it is numeric.
            /// Returns `None` otherwise.
            pub const fn to_u16(self) -> Option<u16> {
                match self {
                    #prop_value_to_u16_arms
                    _ => None,
                }
            }

            pub const fn from_bool(b: bool) -> Self {
                if b {
                    Self::True
                } else {
                    Self::False
                }
            }

            pub const fn to_bool(self) -> Option<bool> {
                match self {
                    Self::True => Some(true),
                    Self::False => Some(false),
                    _ => None,
                }
            }

            /// An array of all property values.
            pub const ALL: [Self; #num_property_names] = [#(Self::#prop_value_variants,)*];
        }

        impl From<bool> for PropValue {
            fn from(b: bool) -> Self {
                Self::from_bool(b)
            }
        }
    };

    let out_path =
        Path::new(&env::var_os("OUT_DIR").context("can't get OUT_DIR env var")?).join("block.rs");

    fs::write(&out_path, &finshed.to_string())?;

    // Format the output for debugging purposes.
    // Doesn't matter if rustfmt is unavailable.
    let _ = Command::new("rustfmt").arg(out_path).output();

    Ok(())
}

struct Block {
    name: String,
    default_state: u16,
    min_state_id: u16,
    max_state_id: u16,
    /// Order of elements in this vec is significant.
    props: Vec<Prop>,
}

struct Prop {
    name: String,
    vals: Vec<String>,
}

fn parse_blocks_json() -> anyhow::Result<Vec<Block>> {
    #[derive(Clone, PartialEq, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct JsonBlock {
        id: u16,
        name: String,
        display_name: String,
        hardness: f64,
        resistance: f64,
        stack_size: u32,
        diggable: bool,
        material: String,
        transparent: bool,
        emit_light: u8,
        filter_light: u8,
        default_state: u16,
        min_state_id: u16,
        max_state_id: u16,
        states: Vec<State>,
        bounding_box: String,
    }

    #[derive(Clone, PartialEq, Debug, Deserialize)]
    #[serde(tag = "type", rename_all = "camelCase")]
    enum State {
        Enum { name: String, values: Vec<String> },
        Int { name: String, values: Vec<String> },
        Bool { name: String },
    }

    let blocks: Vec<JsonBlock> = serde_json::from_str(include_str!("../data/blocks.json"))?;

    Ok(blocks
        .into_iter()
        .map(|b| Block {
            name: b.name,
            default_state: b.default_state,
            min_state_id: b.min_state_id,
            max_state_id: b.max_state_id,
            props: b
                .states
                .into_iter()
                .rev()
                .map(|s| Prop {
                    name: match &s {
                        State::Enum { name, .. } => name.clone(),
                        State::Int { name, .. } => name.clone(),
                        State::Bool { name } => name.clone(),
                    },
                    vals: match &s {
                        State::Enum { values, .. } => values.clone(),
                        State::Int { values, .. } => values.clone(),
                        State::Bool { .. } => vec!["true".to_string(), "false".to_string()],
                    },
                })
                .collect(),
        })
        .collect())
}
