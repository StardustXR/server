# Stardust XR Reference Server

This project is a usable Linux display server that reinvents human-computer interaction for all kinds of XR, from putting 2D/XR apps into various 3D shells for varying uses to SDF-based interaction.

## Prerequisites
1. Cargo
2. CMake
3. EGL+GLES 3.2
4. GLX+Xlib
5. fontconfig
6. dlopen
7. OpenXR Loader (required even if run in flatscreen mode)

## Build
```bash
cargo build
```

## Install
```bash
cargo install
```