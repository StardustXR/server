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

## Test

##### Gnome Graphical Integration Test

- `nix build .#gnome-graphical-test`

   This test uses Nix to reproducibly execute a QEMU virtual machine which
   spawns a full Gnome desktop. It runs `monado-service`, `stardust-xr-server`
   `flatland` underneath of Gnome and then attaches `weston-cliptest` to the
   `flatland` process running underneath of `stardust-xr-server`, the result is
   a screenshot in PNG format that should look like expected. If any process in
   this test produces an exit code above 0, the test will fail, graphical bugs
   should be visible in the screenshot. An example of the result is below.

   ###### Result

   ![image](https://github.com/StardustXR/server/assets/26458780/e21cd039-2528-4568-b20a-ce4abfab6d9b)

##### Everything

`nix flake check` will build every test underneath of the `checks` attribute in the `flake.nix`

