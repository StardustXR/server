use convert_case::{Case, Casing};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, ToTokens};
use split_iter::Splittable;
use stardust_xr_schemas::protocol::*;

fn fold_tokens(a: TokenStream, b: TokenStream) -> TokenStream {
	quote!(#a #b)
}

#[proc_macro]
pub fn codegen_root_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(ROOT_PROTOCOL)
}
#[proc_macro]
pub fn codegen_node_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(NODE_PROTOCOL)
}
#[proc_macro]
pub fn codegen_spatial_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(SPATIAL_PROTOCOL)
}
#[proc_macro]
pub fn codegen_field_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(FIELD_PROTOCOL)
}
#[proc_macro]
pub fn codegen_data_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(DATA_PROTOCOL)
}
#[proc_macro]
pub fn codegen_audio_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(AUDIO_PROTOCOL)
}
#[proc_macro]
pub fn codegen_drawable_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(DRAWABLE_PROTOCOL)
}
#[proc_macro]
pub fn codegen_input_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(INPUT_PROTOCOL)
}
#[proc_macro]
pub fn codegen_item_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(ITEM_PROTOCOL)
}
#[proc_macro]
pub fn codegen_item_camera_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(ITEM_CAMERA_PROTOCOL)
}
#[proc_macro]
pub fn codegen_item_panel_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(ITEM_PANEL_PROTOCOL)
}

fn codegen_protocol(protocol: &'static str) -> proc_macro::TokenStream {
	let protocol = Protocol::parse(protocol).unwrap();
	let interface = protocol
		.interface
		.map(|p| {
			let node_id = p.node_id;
			let node_id = quote! {
				const INTERFACE_NODE_ID: u64 = #node_id;
			};
			let aspect = generate_aspect(&Aspect {
				name: "interface".to_string(),
				description: protocol.description.clone(),
				inherits: vec![],
				members: p.members,
			});
			quote!(#node_id #aspect)
		})
		.unwrap_or_default();
	let custom_enums = protocol
		.custom_enums
		.iter()
		.map(generate_custom_enum)
		.reduce(fold_tokens)
		.unwrap_or_default();
	let custom_unions = protocol
		.custom_unions
		.iter()
		.map(generate_custom_union)
		.reduce(fold_tokens)
		.unwrap_or_default();
	let custom_structs = protocol
		.custom_structs
		.iter()
		.map(generate_custom_struct)
		.reduce(fold_tokens)
		.unwrap_or_default();
	let aspects = protocol
		.aspects
		.iter()
		.map(generate_aspect)
		.reduce(fold_tokens)
		.unwrap_or_default();
	quote!(#custom_enums #custom_unions #custom_structs #aspects #interface).into()
}

fn generate_custom_enum(custom_enum: &CustomEnum) -> TokenStream {
	let name = Ident::new(&custom_enum.name.to_case(Case::Pascal), Span::call_site());
	let description = &custom_enum.description;

	let argument_decls = custom_enum
		.variants
		.iter()
		.map(|a| Ident::new(&a.to_case(Case::Pascal), Span::call_site()).to_token_stream())
		.reduce(|a, b| quote!(#a, #b))
		.unwrap_or_default();

	quote! {
		#[doc = #description]
		#[derive(Debug, Clone, Copy, serde_repr::Deserialize_repr, serde_repr::Serialize_repr)]
		#[repr(u32)]
		pub enum #name {#argument_decls}
	}
}
fn generate_custom_union(custom_union: &CustomUnion) -> TokenStream {
	let name = Ident::new(&custom_union.name.to_case(Case::Pascal), Span::call_site());
	let description = &custom_union.description;

	let option_decls = custom_union
		.options
		.iter()
		.map(generate_union_option)
		.reduce(|a, b| quote!(#a, #b))
		.unwrap_or_default();

	quote! {
		#[doc = #description]
		#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
		#[serde(untagged)]
		pub enum #name {#option_decls}
	}
}
fn generate_union_option(union_option: &UnionOption) -> TokenStream {
	let name = union_option
		.name
		.as_ref()
		.map(|n| n.to_case(Case::Pascal))
		.unwrap_or_else(|| argument_type_option_name(&union_option._type));
	let description = union_option
		.description
		.as_ref()
		.map(|d| quote!(#[doc = #d]))
		.unwrap_or_default();
	let identifier = Ident::new(&name, Span::call_site());
	let _type = generate_argument_type(&union_option._type, false, true);
	quote! (#description #identifier(#_type))
}
fn generate_custom_struct(custom_struct: &CustomStruct) -> TokenStream {
	let name = Ident::new(&custom_struct.name.to_case(Case::Pascal), Span::call_site());
	let description = &custom_struct.description;

	let argument_decls = custom_struct
		.fields
		.iter()
		.map(|a| generate_argument_decl(a, true))
		.map(|d| quote!(pub #d))
		.reduce(|a, b| quote!(#a, #b))
		.unwrap_or_default();

	quote! {
		#[doc = #description]
		#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
		pub struct #name {#argument_decls}
	}
}

fn generate_aspect(aspect: &Aspect) -> TokenStream {
	let description = &aspect.description;
	let (client_members, server_members) = aspect.members.iter().split(|m| m.side == Side::Server);

	let client_mod_name = Ident::new(
		&format!("{}_client", &aspect.name.to_case(Case::Snake)),
		Span::call_site(),
	);
	let client_side_members = client_members
		.map(generate_member)
		.reduce(fold_tokens)
		.map(|t| {
			// TODO: properly import all dependencies
			quote! {
				pub mod #client_mod_name {
					use super::*;
					#t
				}
			}
		})
		.unwrap_or_default();

	let aspect_trait_name = Ident::new(
		&format!("{}Aspect", &aspect.name.to_case(Case::Pascal)),
		Span::call_site(),
	);

	let opcodes = aspect
		.members
		.iter()
		.map(|m| {
			let aspect_name = aspect.name.to_case(Case::ScreamingSnake);
			let member_name = m.name.to_case(Case::ScreamingSnake);
			let name_type = if m.side == Side::Client {
				"CLIENT"
			} else {
				"SERVER"
			};
			let name = Ident::new(
				&format!("{aspect_name}_{member_name}_{name_type}_OPCODE"),
				Span::call_site(),
			);
			let opcode = m.opcode;

			quote!(pub(crate) const #name: u64 = #opcode;)
		})
		.reduce(fold_tokens)
		.unwrap_or_default();
	let alias_info = generate_alias_info(aspect);

	let server_side_members = server_members
		.map(generate_member)
		.reduce(fold_tokens)
		.unwrap_or_default();
	let add_node_members = aspect
		.members
		.iter()
		.filter(|m| m.side == Side::Server)
		.map(generate_handler)
		.reduce(fold_tokens)
		.map(|members| {
			quote! {
				fn add_node_members(node: &crate::nodes::Node) {
					#members
				}
			}
		})
		.unwrap_or_default();
	let server_side_members = quote! {
		#[doc = #description]
		pub trait #aspect_trait_name {
			#add_node_members
			#server_side_members
		}
	};
	quote!(#opcodes #alias_info #client_side_members #server_side_members)
}

fn generate_alias_opcodes(aspect: &Aspect, side: Side, _type: MemberType) -> TokenStream {
	aspect
		.members
		.iter()
		.filter(|m| m.side == side)
		.filter(|m| m._type == _type)
		.map(|m| m.opcode)
		.map(|o| quote!(#o))
		.reduce(|a, b| quote!(#a, #b))
		.unwrap_or_default()
}
fn generate_alias_info(aspect: &Aspect) -> TokenStream {
	let aspect_alias_info_name = Ident::new(
		&format!(
			"{}_ASPECT_ALIAS_INFO",
			aspect.name.to_case(Case::ScreamingSnake)
		),
		Span::call_site(),
	);
	let local_signals = generate_alias_opcodes(aspect, Side::Server, MemberType::Signal);
	let local_methods = generate_alias_opcodes(aspect, Side::Server, MemberType::Method);
	let remote_signals = generate_alias_opcodes(aspect, Side::Client, MemberType::Signal);

	quote! {
		lazy_static::lazy_static! {
			pub static ref #aspect_alias_info_name: crate::nodes::alias::AliasInfo = crate::nodes::alias::AliasInfo {
				server_signals: vec![#local_signals],
				server_methods: vec![#local_methods],
				client_signals: vec![#remote_signals],
			};
		}
	}
}

fn generate_member(member: &Member) -> TokenStream {
	let id = member.opcode;
	let name = Ident::new(&member.name.to_case(Case::Snake), Span::call_site());
	let description = &member.description;

	let side = member.side;
	let _type = member._type;

	let first_args = match member.side {
		Side::Server => {
			quote!(_node: std::sync::Arc<crate::nodes::Node>, _calling_client: std::sync::Arc<crate::core::client::Client>)
		}
		Side::Client => quote!(_node: &crate::nodes::Node),
	};
	let argument_decls = member
		.arguments
		.iter()
		.map(|a| generate_argument_decl(a, member.side == Side::Server))
		.fold(first_args, |a, b| quote!(#a, #b));

	let argument_uses = member
		.arguments
		.iter()
		.map(|a| generate_argument_serialize(&a.name, &a._type, a.optional))
		.reduce(|a, b| quote!(#a, #b))
		.unwrap_or_default();
	let return_type = member
		.return_type
		.as_ref()
		.map(|r| generate_argument_type(&r, false, true))
		.unwrap_or_else(|| quote!(()));

	match (side, _type) {
		(Side::Client, MemberType::Method) => {
			quote! {
				#[doc = #description]
				pub async fn #name(#argument_decls) -> color_eyre::eyre::Result<(#return_type, Vec<std::os::fd::OwnedFd>)> {
					_node.execute_remote_method_typed(#id, &(#argument_uses), vec![]).await
				}
			}
		}
		(Side::Client, MemberType::Signal) => {
			quote! {
				#[doc = #description]
				pub fn #name(#argument_decls) -> color_eyre::eyre::Result<()> {
					let serialized = stardust_xr::schemas::flex::serialize((#argument_uses))?;
					_node.send_remote_signal(#id, serialized)
				}
			}
		}
		(Side::Server, MemberType::Method) => {
			quote! {
				#[doc = #description]
				fn #name(#argument_decls) -> impl std::future::Future<Output = color_eyre::eyre::Result<#return_type>> + Send + 'static;
			}
		}
		(Side::Server, MemberType::Signal) => {
			quote! {
				#[doc = #description]
				fn #name(#argument_decls) -> color_eyre::eyre::Result<()>;
			}
		}
	}
}
fn generate_handler(member: &Member) -> TokenStream {
	let opcode = member.opcode;
	let member_name_ident = Ident::new(&member.name, Span::call_site());

	let argument_names = member
		.arguments
		.iter()
		.map(generate_argument_name)
		.reduce(|a, b| quote!(#a, #b));
	let argument_types = member
		.arguments
		.iter()
		.map(|a| {
			let _type = convert_deserializeable_argument_type(&a._type);
			generate_argument_type(&_type, a.optional, true)
		})
		.reduce(|a, b| quote!(#a, #b));
	// dbg!(&argument_types);
	let deserialize = argument_names
		.clone()
		.zip(argument_types)
		.map(|(argument_names, argument_types)| {
			quote!(let (#argument_names): (#argument_types) = stardust_xr::schemas::flex::deserialize(_message.as_ref())?;)
		})
		.unwrap_or_default();
	let argument_uses = member
		.arguments
		.iter()
		.map(|a| generate_argument_deserialize(&a.name, &a._type, a.optional))
		.reduce(|a, b| quote!(#a, #b))
		.unwrap_or_default();
	match member._type {
		MemberType::Signal => quote! {
			node.add_local_signal(#opcode, |_node, _calling_client, _message| {
				#deserialize
				Self::#member_name_ident(_node, _calling_client.clone(), #argument_uses)
			});
		},
		MemberType::Method => quote! {
			node.add_local_method(#opcode, |_node, _calling_client, _message, _method_response| {
				_method_response.wrap_async(async move {
					#deserialize
					Ok((Self::#member_name_ident(_node, _calling_client.clone(), #argument_uses).await?, Vec::new()))
				});
			});
		},
	}
}
fn generate_argument_name(argument: &Argument) -> TokenStream {
	Ident::new(&argument.name.to_case(Case::Snake), Span::call_site()).to_token_stream()
}

fn convert_deserializeable_argument_type(argument_type: &ArgumentType) -> ArgumentType {
	match argument_type {
		ArgumentType::Node { .. } => ArgumentType::NodeID,
		ArgumentType::Vec(v) => {
			ArgumentType::Vec(Box::new(convert_deserializeable_argument_type(v.as_ref())))
		}
		ArgumentType::Map(v) => {
			ArgumentType::Map(Box::new(convert_deserializeable_argument_type(v.as_ref())))
		}
		f => f.clone(),
	}
}
fn generate_argument_deserialize(
	argument_name: &str,
	argument_type: &ArgumentType,
	optional: bool,
) -> TokenStream {
	let name = Ident::new(&argument_name.to_case(Case::Snake), Span::call_site());
	if let ArgumentType::Node { .. } = argument_type {
		return match optional {
			true => quote!(#name.map(|n| _calling_client.get_node(#argument_name, n)?)),
			false => quote!(_calling_client.get_node(#argument_name, #name)?),
		};
	}
	if optional {
		let mapping = generate_argument_deserialize("o", argument_type, false);
		return quote!(#name.map(|o| Ok::<_, color_eyre::eyre::Report>(#mapping)).transpose()?);
	}

	match argument_type {
		ArgumentType::Color => quote!(color::rgba_linear!(#name[0], #name[1], #name[2], #name[3])),
		ArgumentType::Vec(v) => {
			let mapping = generate_argument_deserialize("a", v, false);
			quote!(#name.into_iter().map(|a| Ok(#mapping)).collect::<color_eyre::eyre::Result<Vec<_>>>()?)
		}
		ArgumentType::Map(v) => {
			let mapping = generate_argument_deserialize("a", v, false);
			quote!(#name.into_iter().map(|(k, a)| Ok((k, #mapping))).collect::<color_eyre::eyre::Result<rustc_hash::FxHashMap<String, _>>>()?)
		}
		_ => quote!(#name),
	}
}
fn generate_argument_serialize(
	argument_name: &str,
	argument_type: &ArgumentType,
	optional: bool,
) -> TokenStream {
	let name = Ident::new(&argument_name.to_case(Case::Snake), Span::call_site());
	match argument_type {
		ArgumentType::Node {
			_type,
			return_id_parameter_name: _,
		} => match optional {
			true => quote!(#name.map(|n| n.get_id())),
			false => quote!(#name.get_id()),
		},
		ArgumentType::Color => quote!([#name.c.r, #name.c.g, #name.c.b, #name.a]),
		ArgumentType::Vec(v) => {
			let mapping = generate_argument_serialize("a", v, false);
			quote!(#name.into_iter().map(|a| Ok(#mapping)).collect::<color_eyre::eyre::Result<Vec<_>>>()?)
		}
		ArgumentType::Map(v) => {
			let mapping = generate_argument_serialize("a", v, false);
			quote!(#name.into_iter().map(|(k, a)| Ok((k, #mapping))).collect::<color_eyre::eyre::Result<rustc_hash::FxHashMap<String, _>>>()?)
		}
		_ => quote!(#name),
	}
}
fn generate_argument_decl(argument: &Argument, owned_values: bool) -> TokenStream {
	let name = Ident::new(&argument.name.to_case(Case::Snake), Span::call_site());
	let mut _type = generate_argument_type(&argument._type, argument.optional, owned_values);
	quote!(#name: #_type)
}
fn argument_type_option_name(argument_type: &ArgumentType) -> String {
	match argument_type {
		ArgumentType::Empty => "Empty".to_string(),
		ArgumentType::Bool => "Bool".to_string(),
		ArgumentType::Int => "Int".to_string(),
		ArgumentType::UInt => "UInt".to_string(),
		ArgumentType::Float => "Float".to_string(),
		ArgumentType::Vec2(_) => "Vec2".to_string(),
		ArgumentType::Vec3(_) => "Vec3".to_string(),
		ArgumentType::Quat => "Quat".to_string(),
		ArgumentType::Mat4 => "Mat4".to_string(),
		ArgumentType::Color => "Color".to_string(),
		ArgumentType::String => "String".to_string(),
		ArgumentType::Bytes => "Bytes".to_string(),
		ArgumentType::Vec(v) => format!("{}Vector", argument_type_option_name(&v)),
		ArgumentType::Map(m) => format!("{}Map", argument_type_option_name(&m)),
		ArgumentType::NodeID => "Node ID".to_string(),
		ArgumentType::Datamap => "Datamap".to_string(),
		ArgumentType::ResourceID => "ResourceID".to_string(),
		ArgumentType::Enum(e) => e.clone(),
		ArgumentType::Union(u) => u.clone(),
		ArgumentType::Struct(s) => s.clone(),
		ArgumentType::Node { _type, .. } => _type.clone(),
	}
}
fn generate_argument_type(
	argument_type: &ArgumentType,
	optional: bool,
	owned: bool,
) -> TokenStream {
	let _type = match argument_type {
		ArgumentType::Empty => quote!(()),
		ArgumentType::Bool => quote!(bool),
		ArgumentType::Int => quote!(i32),
		ArgumentType::UInt => quote!(u32),
		ArgumentType::Float => quote!(f32),
		ArgumentType::Vec2(t) => {
			let t = generate_argument_type(&t, false, true);
			quote!(stardust_xr::values::Vector2<#t>)
		}
		ArgumentType::Vec3(t) => {
			let t = generate_argument_type(&t, false, true);
			quote!(stardust_xr::values::Vector3<#t>)
		}
		ArgumentType::Quat => quote!(stardust_xr::values::Quaternion),
		ArgumentType::Mat4 => quote!(stardust_xr::values::Mat4),
		ArgumentType::Color => quote!(stardust_xr::values::Color),
		ArgumentType::Bytes => {
			if !owned {
				quote!(&[u8])
			} else {
				quote!(Vec<u8>)
			}
		}
		ArgumentType::String => {
			if !owned {
				quote!(&str)
			} else {
				quote!(String)
			}
		}
		ArgumentType::Vec(t) => {
			let t = generate_argument_type(&t, false, true);
			if !owned {
				quote!(&[#t])
			} else {
				quote!(Vec<#t>)
			}
		}
		ArgumentType::Map(t) => {
			let t = generate_argument_type(&t, false, true);

			if !owned {
				quote!(&stardust_xr::values::Map<String, #t>)
			} else {
				quote!(stardust_xr::values::Map<String, #t>)
			}
		}
		ArgumentType::NodeID => quote!(u64),
		ArgumentType::Datamap => {
			if !owned {
				quote!(&stardust_xr::values::Datamap)
			} else {
				quote!(stardust_xr::values::Datamap)
			}
		}
		ArgumentType::ResourceID => {
			if !owned {
				quote!(&stardust_xr::values::ResourceID)
			} else {
				quote!(stardust_xr::values::ResourceID)
			}
		}
		ArgumentType::Enum(e) => {
			let enum_name = Ident::new(&e.to_case(Case::Pascal), Span::call_site());
			quote!(#enum_name)
		}
		ArgumentType::Union(u) => {
			let union_name = Ident::new(&u.to_case(Case::Pascal), Span::call_site());
			quote!(#union_name)
		}
		ArgumentType::Struct(s) => {
			let struct_name = Ident::new(&s.to_case(Case::Pascal), Span::call_site());
			if !owned {
				quote!(&#struct_name)
			} else {
				quote!(#struct_name)
			}
		}
		ArgumentType::Node {
			_type,
			return_id_parameter_name: _,
		} => {
			if !owned {
				quote!(&std::sync::Arc<crate::nodes::Node>)
			} else {
				quote!(std::sync::Arc<crate::nodes::Node>)
			}
		}
	};

	if optional {
		quote!(Option<#_type>)
	} else {
		_type
	}
}
