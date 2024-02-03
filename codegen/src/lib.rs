use convert_case::{Case, Casing};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, ToTokens};
use split_iter::Splittable;
use stardust_xr_schemas::protocol::*;

fn fold_tokens(a: TokenStream, b: TokenStream) -> TokenStream {
	quote!(#a #b)
}

// #[proc_macro]
// pub fn codegen_root_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
// 	codegen_protocol(ROOT_PROTOCOL)
// }
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
pub fn codegen_zone_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(ZONE_PROTOCOL)
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
pub fn codegen_drawable_lines_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(DRAWABLE_LINES_PROTOCOL)
}
#[proc_macro]
pub fn codegen_drawable_model_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(DRAWABLE_MODEL_PROTOCOL)
}
#[proc_macro]
pub fn codegen_drawable_text_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
	codegen_protocol(DRAWABLE_TEXT_PROTOCOL)
}
// #[proc_macro]
// pub fn codegen_input_protocol(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
// 	codegen_protocol(INPUT_PROTOCOL)
// }

fn codegen_protocol(protocol: &'static str) -> proc_macro::TokenStream {
	let protocol = Protocol::parse(protocol).unwrap();
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
	// let aspects = protocol
	// 	.aspects
	// 	.iter()
	// 	.map(generate_aspect)
	// 	.reduce(fold_tokens)
	// 	.unwrap_or_default();
	// let nodes = protocol
	// 	.nodes
	// 	.iter()
	// 	.map(generate_node)
	// 	.reduce(fold_tokens)
	// 	.unwrap_or_default();
	// let interfaces = protocol
	// 	.interfaces
	// 	.iter()
	// 	.map(generate_interface)
	// 	.reduce(fold_tokens)
	// 	.unwrap_or_default();
	// quote!(#custom_enums #custom_unions #custom_structs #aspects #nodes #interfaces).into()
	quote!(#custom_enums #custom_unions #custom_structs).into()
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
		#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
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
	let _type = generate_argument_type(&union_option._type, true);
	quote! (#description #identifier(#_type))
}
fn argument_type_option_name(argument_type: &ArgumentType) -> String {
	match argument_type {
		ArgumentType::Bool => "Bool".to_string(),
		ArgumentType::Int => "Int".to_string(),
		ArgumentType::UInt => "UInt".to_string(),
		ArgumentType::Float => "Float".to_string(),
		ArgumentType::Vec2 => "Vec2".to_string(),
		ArgumentType::Vec3 => "Vec3".to_string(),
		ArgumentType::Quat => "Quat".to_string(),
		ArgumentType::Color => "Color".to_string(),
		ArgumentType::String => "String".to_string(),
		ArgumentType::Bytes => "Bytes".to_string(),
		ArgumentType::Vec(v) => format!("{}Vector", argument_type_option_name(&v)),
		ArgumentType::Map(m) => format!("{}Map", argument_type_option_name(&m)),
		ArgumentType::Datamap => "Datamap".to_string(),
		ArgumentType::ResourceID => "ResourceID".to_string(),
		ArgumentType::Enum(e) => e.clone(),
		ArgumentType::Union(u) => u.clone(),
		ArgumentType::Struct(s) => s.clone(),
		ArgumentType::Node { _type, .. } => _type.clone(),
	}
}
fn generate_custom_struct(custom_struct: &CustomStruct) -> TokenStream {
	let name = Ident::new(&custom_struct.name.to_case(Case::Pascal), Span::call_site());
	let description = &custom_struct.description;

	let argument_decls = custom_struct
		.fields
		.iter()
		.map(|a| generate_argument_decl(a, true))
		.reduce(|a, b| quote!(#a, #b))
		.unwrap_or_default();

	quote! {
		#[doc = #description]
		#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
		pub struct #name {#argument_decls}
	}
}

fn generate_interface(interface: &Interface) -> TokenStream {
	interface
		.members
		.iter()
		.map(generate_member)
		.reduce(fold_tokens)
		.unwrap_or_default()
}

fn generate_node(node: &Node) -> TokenStream {
	let node_name = Ident::new(&node.name, Span::call_site());
	let description = &node.description;

	let aspects = node
		.aspects
		.iter()
		.map(|a| {
			let aspect_name = Ident::new(&format!("{a}Aspect"), Span::call_site());
			quote!(impl #aspect_name for #node_name {})
		})
		.reduce(fold_tokens)
		.unwrap_or_default();

	quote! {
		#[doc = #description]
		#[derive(Debug)]
		pub struct #node_name (crate::node::Node);
		impl crate::node::NodeType for #node_name {
			fn node(&self) -> &crate::node::Node {
				&self.0
			}
			fn alias(&self) -> Self {
				#node_name(self.0.alias())
			}
			fn from_path(client: &std::sync::Arc<crate::client::Client>, path: String, destroyable: bool) -> Self {
				#node_name(crate::node::Node::from_path(client, path, destroyable))
			}
		}
		impl serde::Serialize for #node_name {
			fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
				let node_path = self.0.get_path().map_err(|e| serde::ser::Error::custom(e))?;
				serializer.serialize_str(&node_path)
			}
		}
		#aspects
	}
}

fn generate_aspect(aspect: &Aspect) -> TokenStream {
	let description = &aspect.description;
	let (client_members, server_members) = aspect.members.iter().split(|m| m.side == Side::Server);

	let aspect_handler_name = Ident::new(
		&format!("{}Handler", &aspect.name.to_case(Case::Pascal)),
		Span::call_site(),
	);
	let client_side_members = client_members
		.map(generate_member)
		.reduce(fold_tokens)
		.map(|t| {
			quote! {
				#[doc = #description]
				pub trait #aspect_handler_name: Send + Sync + 'static {
					#t
				}
			}
		})
		.unwrap_or_default();
	let aspect_wrap = aspect
		.members
		.iter()
		.filter(|m| m.side == Side::Client)
		.map(generate_handler)
		.reduce(fold_tokens).map(|handlers| {
			quote! {
				#[must_use = "Dropping this handler wrapper would immediately drop the handler"]
				fn wrap<H: #aspect_handler_name>(self, handler: H) -> NodeResult<crate::HandlerWrapper<Self, H>> {
					self.wrap_raw(std::sync::Arc::new(parking_lot::Mutex::new(handler)))
				}
				#[must_use = "Dropping this handler wrapper would immediately drop the handler"]
				fn wrap_raw<H: #aspect_handler_name>(self, handler: std::sync::Arc<parking_lot::Mutex<H>>) -> NodeResult<crate::HandlerWrapper<Self, H>> {
					let handler_wrapper = crate::HandlerWrapper::new_raw(self, handler);
					#handlers
					Ok(handler_wrapper)
				}
			}
		}).unwrap_or_default();

	let aspect_trait_name = Ident::new(
		&format!("{}Aspect", &aspect.name.to_case(Case::Pascal)),
		Span::call_site(),
	);
	let server_side_members = server_members
		.map(generate_member)
		.reduce(fold_tokens)
		.unwrap_or_default();
	let server_side_members = quote! {
		#[doc = #description]
		pub trait #aspect_trait_name: crate::node::NodeType {
			#aspect_wrap
			#server_side_members
		}
	};
	quote!(#client_side_members #server_side_members)
}

fn generate_member(member: &Member) -> TokenStream {
	let name_str = &member.name;
	let name = Ident::new(&member.name.to_case(Case::Snake), Span::call_site());
	let description = &member.description;

	let side = member.side;
	let _type = member._type;

	let first_arg = if member.interface_path.is_some() {
		quote!(client: &std::sync::Arc<crate::client::Client>)
	} else {
		if member.side == Side::Server {
			quote!(&self)
		} else {
			quote!(&mut self)
		}
	};
	let argument_decls = member
		.arguments
		.iter()
		.map(|a| generate_argument_decl(a, member.side == Side::Client))
		.fold(first_arg, |a, b| quote!(#a, #b));

	let argument_uses = member
		.arguments
		.iter()
		.map(|a| generate_argument_serialize(&a.name, &a._type, a.optional))
		.reduce(|a, b| quote!(#a, #b))
		.unwrap_or_default();
	let return_type = member
		.return_type
		.as_ref()
		.map(|r| generate_argument_type(&r, true))
		.unwrap_or_else(|| quote!(()));

	match (side, _type) {
		(Side::Server, MemberType::Method) => {
			let body = if let Some(interface_path) = &member.interface_path {
				quote! {
					let data = stardust_xr::schemas::flex::serialize(&(#argument_uses))?;
					let result = client.message_sender_handle.method(#interface_path, #name_str, &data, Vec::new())?.await?;
					Ok(stardust_xr::schemas::flex::deserialize(&result.into_message())?)
				}
			} else {
				quote! {
					self.node().execute_remote_method(#name_str, &(#argument_uses)).await
				}
			};
			quote! {
				#[doc = #description]
				async fn #name(#argument_decls) -> crate::node::NodeResult<#return_type> {
					#body
				}
			}
		}
		(Side::Server, MemberType::Signal) => {
			let mut body = if let Some(interface_path) = &member.interface_path {
				quote! {
					client.message_sender_handle.signal(#interface_path, #name_str, &stardust_xr::schemas::flex::serialize(&(#argument_uses))?, Vec::new())
				}
			} else {
				quote! {
					self.node().send_remote_signal(#name_str, &(#argument_uses))
				}
			};
			body = if let Some(ArgumentType::Node {
				_type: _,
				return_info,
			}) = &member.return_type
			{
				if let Some(return_info) = return_info {
					let parent = &return_info.parent;
					let name_argument = Ident::new(&return_info.name_argument, Span::call_site());
					let get_client = if member.interface_path.is_some() {
						quote!(client)
					} else {
						quote!(self.node().client()?)
					};
					quote! {
						#body?;
						Ok(<#return_type as crate::node::NodeType>::from_parent_name(#get_client, #parent, &#name_argument, true))
					}
				} else {
					quote! {
						Ok(#body?)
					}
				}
			} else {
				quote! {
					Ok(#body?)
				}
			};
			quote! {
				#[doc = #description]
				fn #name(#argument_decls) -> crate::node::NodeResult<#return_type> {
					#body
				}
			}
		}
		(Side::Client, MemberType::Method) => {
			quote! {
				#[doc = #description]
				fn #name(#argument_decls) -> crate::node::MethodResult<#return_type>;
			}
		}
		(Side::Client, MemberType::Signal) => {
			quote! {
				#[doc = #description]
				fn #name(#argument_decls);
			}
		}
	}
}
fn generate_handler(member: &Member) -> TokenStream {
	let name = &member.name;
	let name_ident = Ident::new(&name, Span::call_site());

	let argument_names = member
		.arguments
		.iter()
		.map(generate_argument_name)
		.reduce(|a, b| quote!(#a, #b));
	let argument_types = member
		.arguments
		.iter()
		.map(|a| &a._type)
		.map(convert_deserializeable_argument_type)
		.map(|a| generate_argument_type(&a, true))
		.reduce(|a, b| quote!(#a, #b));
	// dbg!(&argument_types);
	let deserialize = argument_names
		.clone()
		.zip(argument_types)
		.map(|(argument_names, argument_types)| {
			quote!(let (#argument_names): (#argument_types) = stardust_xr::schemas::flex::deserialize(_data)?;)
		})
		.unwrap_or_default();
	let argument_uses = member
		.arguments
		.iter()
		.map(|a| generate_argument_deserialize(&a.name, &a._type, a.optional))
		.fold(TokenStream::default(), |a, b| quote!(#a, #b));
	let handler_wrapper_method_name = match member._type {
		MemberType::Signal => quote!(add_handled_signal),
		MemberType::Method => quote!(add_handled_method),
	};

	quote! {
		handler_wrapper.#handler_wrapper_method_name(#name, |_node, _handler, _data, _fds| {
			#deserialize
			let _client = _node.client()?;
			let mut _handler_lock = _handler.lock();
			Ok(H::#name_ident(&mut *_handler_lock #argument_uses))
		})?;
	}
}
fn generate_argument_name(argument: &Argument) -> TokenStream {
	Ident::new(&argument.name.to_case(Case::Snake), Span::call_site()).to_token_stream()
}
fn generate_argument_deserialize(
	argument_name: &str,
	argument_type: &ArgumentType,
	optional: bool,
) -> TokenStream {
	let name = Ident::new(&argument_name.to_case(Case::Snake), Span::call_site());
	match argument_type {
		ArgumentType::Node {
			_type,
			return_info: _,
		} => {
			let node_type = Ident::new(&_type.to_case(Case::Pascal), Span::call_site());
			match optional {
				true => quote!(#name.map(|n| #node_type::from_path(&_client, n, false))),
				false => quote!(#node_type::from_path(&_client, #name, false)),
			}
		}
		ArgumentType::Color => quote!(color::rgba_linear!(#name[0], #name[1], #name[2], #name[3])),
		ArgumentType::Vec(v) => {
			let mapping = generate_argument_deserialize("a", v, false);
			quote!(#name.iter().map(|a| Ok(#mapping)).collect::<Result<Vec<_>, crate::node::NodeError>>()?)
		}
		ArgumentType::Map(v) => {
			let mapping = generate_argument_deserialize("a", v, false);
			quote!(#name.iter().map(|(k, a)| Ok((k, #mapping))).collect::<Result<rustc_hash::FxHashMap<String, _>, crate::node::NodeError>>()?)
		}
		_ => quote!(#name),
	}
}
fn convert_deserializeable_argument_type(argument_type: &ArgumentType) -> ArgumentType {
	match argument_type {
		ArgumentType::Node { .. } => ArgumentType::String,
		f => f.clone(),
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
			return_info: _,
		} => match optional {
			true => quote!(#name.map(|n| n.node().get_path()).transpose()?),
			false => quote!(#name.node().get_path()?),
		},
		ArgumentType::Color => quote!([#name.c.r, #name.c.g, #name.c.b, #name.a]),
		ArgumentType::Vec(v) => {
			let mapping = generate_argument_serialize("a", v, false);
			quote!(#name.iter().map(|a| Ok(#mapping)).collect::<Result<Vec<_>, crate::node::NodeError>>()?)
		}
		ArgumentType::Map(v) => {
			let mapping = generate_argument_serialize("a", v, false);
			quote!(#name.iter().map(|(k, a)| Ok((k, #mapping))).collect::<Result<rustc_hash::FxHashMap<String, _>, crate::node::NodeError>>()?)
		}
		_ => quote!(#name),
	}
}
fn generate_argument_decl(argument: &Argument, returned: bool) -> TokenStream {
	let name = Ident::new(&argument.name.to_case(Case::Snake), Span::call_site());
	let mut _type = generate_argument_type(&argument._type, returned);
	if argument.optional {
		_type = quote!(Option<#_type>);
	}
	quote!(#name: #_type)
}
fn generate_argument_type(argument_type: &ArgumentType, owned: bool) -> TokenStream {
	match argument_type {
		ArgumentType::Bool => quote!(bool),
		ArgumentType::Int => quote!(i32),
		ArgumentType::UInt => quote!(u32),
		ArgumentType::Float => quote!(f32),
		ArgumentType::Vec2 => quote!(mint::Vector2<f32>),
		ArgumentType::Vec3 => quote!(mint::Vector3<f32>),
		ArgumentType::Quat => quote!(mint::Quaternion<f32>),
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
			let t = generate_argument_type(&t, true);
			if !owned {
				quote!(&[#t])
			} else {
				quote!(Vec<#t>)
			}
		}
		ArgumentType::Map(t) => {
			let t = generate_argument_type(&t, true);

			if !owned {
				quote!(&rustc_hash::FxHashMap<String, #t>)
			} else {
				quote!(rustc_hash::FxHashMap<String, #t>)
			}
		}
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
			quote!(#struct_name)
		}
		ArgumentType::Node {
			_type,
			return_info: _,
		} => {
			if !owned {
				let aspect = Ident::new(
					&format!("{}Aspect", _type.to_case(Case::Pascal)),
					Span::call_site(),
				);
				quote!(&impl #aspect)
			} else {
				let node = Ident::new(&_type.to_case(Case::Pascal), Span::call_site());
				quote!(#node)
			}
		}
	}
}
