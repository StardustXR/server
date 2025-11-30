# WARP.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

## Common Commands

- **Build**: `cargo build` (Use `--release` for performance testing/running, as validation layers in debug can be slow)
- **Run (Flatscreen)**: `cargo run -- -f` (Forces flatscreen mode for debugging without headset)
- **Run (VR/AR)**: `cargo run`
- **Test**: `cargo test`
- **Lint**: `cargo clippy`
- **Run Single Test**: `cargo test -- <test_name>`

## Architecture

Stardust XR Server is a display server for XR, built using Rust and the Bevy game engine.

### High-Level Structure

- **Core Loop**: The application runs a Bevy app loop (`src/main.rs`). It integrates `tokio` for async networking alongside Bevy's ECS.
- **Client/Server**:
    - Clients connect via Unix domain sockets.
    - `src/core/client.rs` manages client connections, message dispatching, and lifecycle.
    - The server communicates with clients using a custom protocol defined in `stardust-xr` and generated via the `codegen` crate.
- **Scenegraph**:
    - The world is composed of **Nodes** (`src/nodes/mod.rs`).
    - Nodes have **Aspects** which define their capabilities (e.g., `Spatial`, `Drawable`, `Input`).
    - `src/nodes/` contains implementations for different node types.
- **XR Integration**:
    - Uses `bevy_mod_openxr` for OpenXR support.
    - `src/objects/` handles XR-specific objects like HMDs, controllers, and playspaces.
- **Wayland Integration**:
    - The `wayland` feature (`src/wayland/`) implements a Wayland compositor to allow running standard 2D Linux applications within the XR space.

### Key Directories

- `src/core`: Core infrastructure (clients, tasks, entity handles).
- `src/nodes`: Scenegraph node implementations (Spatial, Model, Text, Audio, etc.).
- `src/objects`: System objects (HMD, Input, PlaySpace).
- `src/wayland`: Wayland compositor implementation (requires `wayland` feature).
- `codegen`: Procedural macros for generating protocol-related code.

### Development Notes

- **Async/Sync Bridge**: The codebase bridges `tokio` async tasks with Bevy's synchronous ECS. Watch out for deadlocks or blocking the main thread.
- **Protocol Generation**: Changes to the client/server protocol often involve the `codegen` crate.
- **Flatscreen Mode**: Use `cargo run -- -f` to develop and test without needing an active VR headset.
