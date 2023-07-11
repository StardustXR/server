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
The latest stable server is automatically built to an appimage at https://github.com/StardustXR/server/releases for easy testing.

## Usage

First, try running `cargo run` in a terminal window. If a headset is plugged in and OpenXR is working no window will show up. However, the headset should show the same things as the window that opens:

![A pitch black void with a single bleach white hand in the middle](/img/xr_mode_windowed_blank.png)

Stardust won't do anything interesting without clients! Try some from https://github.com/StardustXR.

### Default Sky

You can set a default skytex/skylight by putting your favorite HDRI equirectangular sky in `~/.config/stardust/skytex.hdr`. Certain clients can override this.

Flatscreen mode when the default skybox is [Zhengyang Gate](https://polyhaven.com/a/zhengyang_gate):
![A pitch black window representing Stardust in flatscreen mode](/img/flatscreen_3.png)

### Windowed Mode

If the stardust server can't connect to an OpenXR runtime or you force it into flatscreen mode with `-f`, the server will show in a window.
![A black void representing Stardust in XR mode with a hand skeleton in the middle](/img/flatscreen_2.png)

You can navigate around by right click + dragging to look around, Shift+W/A/S/D/Q/E to move. If you have a virtual hand, left click pinches, right click points, both make a fist.

### Flags
#### Flatscreen (-f)

The server will show up in windowed mode no matter what with your mouse pointer being turned into a 3D pointer. Keyboard input will be sent to whatever your mouse is hovering over like visionOS simulator.
Flatscreen mode upon initial startup:
![A pitch black window representing Stardust in flatscreen mode](/img/flatscreen_1.png)

#### Overlay (-o \<PRIORITY>)

The server will, if in XR mode, be overlaid using the OpenXR overlay extension with the given priority.

#### Disable controller (--disable-controller)

Some runtimes such as Monado may emulate a controller using a hand, and this messes with Stardust's input system. Set this flag to ignore the controllers that the OpenXR runtime provides.

#### Execute (-e </path/to/executable>)

When wayland and OpenXR and such are initialized, run the given executable (such as a bash script) with all the environment variables needed to connect all clients of any type to the server. If not set, the server will run the executable at `~/.config/stardust/startup` if it exists. This is how stardust desktop environments can be made.

#### Help (-h, --help)

help

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