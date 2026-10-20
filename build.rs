use gluon_ipc_codegen::Derives;

fn main() {
	gluon_ipc_codegen::helpers::gen_single_module(
		"gluon/org.stardustxr.ServerDebug.gluon",
		Derives::empty(),
		&[],
		false,
		"src/debug_protocol.rs",
	);
}
