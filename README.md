# Stardust XR Server

Stardust XR is a display server for VR and AR headsets on Linux-based systems. [Stardust provides a 3D environment](https://www.youtube.com/watch?v=v2WblwbaLaA), where anything from 2D windows (including your existing apps!), to 3D apps built from objects, can exist together in physical space.  

![workflow](/img/workflow.png)

## Core Dependencies 
| Functionality      | Ubuntu (apt)                                        | Fedora (dnf)                                                         | Arch Linux (pacman)                        |
| ------------------ | --------------------------------------------------- | -------------------------------------------------------------------- | ------------------------------------------ |
| **EGL / GL**       | libegl-dev, libgl-dev, libgbm-dev, libdrm-dev       | mesa-libEGL-devel, mesa-libGL-devel, mesa-libgbm-devel, libdrm-devel | mesa (includes development files), libdrm  |
| **GLES 3.2**       | libgles2-mesa-dev                                   | mesa-libGLES-devel                                                   | mesa (provides GLES libraries and headers) |
| **X11**            | libx11-dev, libxcb1-dev, libxfixes-dev, libxau-dev  | libX11-devel, libxcb-devel, libXfixes-devel, libXau-devel            | libx11, libxcb, libxfixes, libxau          |
| **Font Rendering** | libfontconfig1-dev, libfreetype6-dev                | fontconfig-devel, freetype-devel                                     | fontconfig, freetype2                      |
| **Compression**    | zlib1g-dev, libbz2-dev, libbrotli-dev, liblzma-dev  | zlib-devel, bzip2-devel, brotli-devel, xz-devel                      | zlib, bzip2, brotli, xz                    |
| **Text Rendering** | libharfbuzz-dev, libgraphite2-dev                   | harfbuzz-devel, graphite2-devel                                      | harfbuzz, graphite                         |
| **XML / Parsing**  | libxml2-dev, libexpat1-dev, libpcre2-dev            | libxml2-devel, expat-devel, pcre2-devel                              | libxml2, expat, pcre2                      |
| **Standard C++**   | libstdc++-dev-12                                    | libstdc++-devel                                                      | gcc-libs (includes libstdc++)              |
| **XKB / Keyboard** | libxkbcommon-dev, libxkbcommon-x11-dev              | libxkbcommon-devel, libxkbcommon-x11-devel                           | libxkbcommon, libxkbcommon-x11             |
| **Core System**    | libglib2.0-dev                                      | glib2-devel                                                          | glib2                                      |
| **PNG Support**    | libpng-dev                                          | libpng-devel                                                         | libpng                                     |
| **Cargo (Rust)**   | cargo                                               | cargo                                                                | cargo (part of the rust package)           |
| **CMake**          | cmake                                               | cmake                                                                | cmake                                      |
| **dlopen (glibc)** | libc6-dev                                           | glibc-devel                                                          | glibc                                      |
| **OpenXR Loader**  | libopenxr-dev, libopenxr-loader1, libopenxr1-monado | openxr-devel                                                         | openxr                                     |

Command line installation of core & dynamic dependencies are provided below:
<details>
<summary>Ubuntu/Debian</summary> 
  <pre><code class="language-bash">
  sudo apt update && sudo apt install \
  build-essential \
  cargo \
  cmake \
  libxkbcommon-dev libxkbcommon-x11-dev libstdc++-dev libx11-dev libxfixes-dev \
  libegl-dev libgbm-dev libfontconfig1-dev libxcb1-dev libgl-dev libdrm-dev \
  libexpat1-dev libfreetype6-dev libxml2-dev libxau-dev zlib1g-dev libbz2-dev \
  libpng-dev libharfbuzz-dev libbrotli-dev liblzma-dev libglib2.0-dev \
  libgraphite2-dev libpcre2-dev
  </code></pre>
</details>

<details>
<summary>Fedora</summary> 
  <pre><code class="language-bash">
  sudo apt update && sudo apt install \
  libxkbcommon-dev libxkbcommon-x11-dev libstdc++-dev libx11-dev libxfixes-dev \
  libegl-dev libgbm-dev libfontconfig1-dev libxcb1-dev libgl-dev libdrm-dev \
  libexpat1-dev libfreetype6-dev libxml2-dev libxau-dev zlib1g-dev libbz2-dev \
  libpng-dev libharfbuzz-dev libbrotli-dev liblzma-dev libglib2.0-dev \
  libgraphite2-dev libpcre2-dev
  </code></pre>
</details>


<details>
<summary>Arch Linux</summary> 
  <pre><code class="language-bash">
  sudo pacman -Syu --needed \
  cargo \
  cmake \
  libxkbcommon libxkbcommon-x11 libx11 libxfixes mesa fontconfig libxcb \
  libdrm expat freetype2 libxml2 libxau zlib bzip2 libpng harfbuzz brotli \
  xz glib2 graphite pcre2
  </code></pre>
</details>

## Installation

More detailed instructions and walkthroughs are provided at https://www.stardustxr.org

The [Terra Repository](https://terra.fyralabs.com/) is required, and comes pre-installed with [Ultramarine Linux](https://ultramarine-linux.org/). Other Fedora Editions and derivatives can directly install terra-release:

```bash
sudo dnf install --nogpgcheck --repofrompath 'terra,https://repos.fyralabs.com/terra$releasever' terra-release
```

For a full installation of the Stardust XR server *and* a selected group of clients, run:

```bash
sudo dnf group install stardust-xr
```

## Manual Build
We've provided a manual installation script [here](https://github.com/cyberneticmelon/usefulscripts/blob/main/stardustxr_setup.sh) that clones and builds the Stardust XR server along with a number of other clients from their respective repositories, and provides a startup script for automatically launching some clients.

After cloning the repository
```bash
cargo build
```

## Usage
> [!NOTE]
> For help with setting up an XR headset on linux, visit https://stardustxr.org/docs/get-started/setup-openxr


The **Stardust XR Server** is a server that runs clients, so without any running, you will see a black screen. If you only have the server installed, we recommend also cloning and building the following clients to start: [Flatland](https://github.com/StardustXR/flatland), which allows normal 2D apps to run in Stardust, [Protostar](https://github.com/StardustXR/protostar), which contains Hexagon Launcher, an app launcher menu, and [Black Hole](https://github.com/StardustXR/black-hole) to quickly tuck away your objects and apps (kind of like desktop peek on Windows).

First, try running `cargo run -- -f` in a terminal window to check out flatscreen mode, (or `stardust-xr-server -f` / `stardust-xr-server_dev -f` if you installed via dnf or the manual installation script, respectively, as they provide symlinks.)

If there aren't already any clients running, you'll need to manually launch them by either navigating to their repositories and running `cargo run`, or running them via their names if you installed via dnf or the manual installation script, such as `flatland`, `hexagon_launcher`, etc.

> [!IMPORTANT]
> [Flatland](https://github.com/StardustXR/flatland) must be running for 2D apps to launch. 

### Startup Script
A startup script can be created at `~/.config/stardust/startup` that will launch specified settings and clients/applications, an example of which is shown [here](https://github.com/cyberneticmelon/usefulscripts/blob/main/startup). If you used the [installation script](https://github.com/cyberneticmelon/usefulscripts/blob/main/stardustxr_setup.sh), one will have already been made for you. This allows wide flexibility of what clients to launch upon startup (and, for example, *where*, using the [Gravity](https://github.com/StardustXR/gravity) client to specify X Y and Z co-ordinates).

### Flatscreen Navigation
A video guide showcasing flatscreen controls is available [here](https://www.youtube.com/watch?v=JCYecSlKlDI)  

To move around, hold down `Shift + W A S D`, with `Q` for moving down and `E` for moving up.
![wasd](https://github.com/StardustXR/website/blob/main/static/img/updated_flat_wasd.GIF)

To look around, hold down `Shift + Right` Click while moving the mouse. 
![updated_look](https://github.com/StardustXR/website/blob/main/static/img/updated_flat_look.GIF)

To drag applications out of the app launcher, hold down `Shift + ~`
![updated_drag](https://github.com/StardustXR/website/blob/main/static/img/updated_flat_drag.GIF)

### XR Navigation
A video guide showcasing XR controls is available [here](https://www.youtube.com/watch?v=RbxFq6JjliA)  

**Quest 3 Hand tracking**:
Pinch to drag and drop, grasp with full hand for grabbing, point and click with pointer finger to click or pinch from a distance  

![hand_pinching](https://github.com/StardustXR/website/blob/main/static/img/hand_pinching.GIF)

**Quest 3 Controller**:
Grab with the grip buttons, click by touching the tip of the cones or by using the trigger from a distance  

![controller_click](https://github.com/StardustXR/website/blob/main/static/img/controller_click.GIF)
