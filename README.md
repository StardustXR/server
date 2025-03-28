# Stardust XR Server

Stardust XR is a display server for VR and AR headsets on Linux-based systems. [Stardust provides a 3D environment](https://www.youtube.com/watch?v=v2WblwbaLaA), where anything from 2D windows (including your existing apps!), to 3D apps built from objects, can exist together in physical space.  

![workflow](/img/workflow.png)

## Core Dependencies 
| **Dependency**              | **Ubuntu/Debian**                                                                               | **Arch Linux**                                    | **Fedora**                                                  |
|-----------------------------|-------------------------------------------------------------------------------------------------|---------------------------------------------------|-------------------------------------------------------------|
| **Cargo**                   | `cargo`                                                                                       | `cargo` | `cargo` |
| **CMake**                   | `cmake`                                                                                       | `cmake`                                           | `cmake`                                                     |
| **EGL+GLES 3.2**            | `libegl1-mesa-dev`, `libgles2-mesa-dev`                                                         | `mesa` *(provides EGL/GLES libraries and headers)* | `mesa-libEGL-devel`, `mesa-libGLES-devel`                     |
| **GLX+Xlib**                | `libx11-dev`, `libxfixes-dev`, `libxcb1-dev`, `libgl1-mesa-dev`, `libxkbcommon-dev`              | `libx11`, `libxfixes`, `libxcb` *(and GLX via mesa)*| `libX11-devel`, `libXfixes-devel`, `libxcb-devel`, `mesa-libGL-devel` *(or equivalent)* |
| **fontconfig**              | `libfontconfig1-dev`                                                                            | `fontconfig`                                      | `fontconfig-devel`                                          |
| **dlopen** (glibc function) | Provided by `libc6-dev` (part of the core C library)                                            | Provided by `glibc` *(included in base-devel)*    | Provided by `glibc-devel`                                     |
| **OpenXR Loader**           | `libopenxr-loader1`, `libopenxr-dev`, `libopenxr1-monado`                                       | `openxr`                                          | `openxr-devel`                                              |

Command line installation of core & dynamic dependencies are provided below:
<details>
<summary>Ubuntu/Debian</summary> 
  <pre><code class="language-bash">
  sudo apt-get update && sudo apt-get install -y \
  build-essential \
  cargo \
  cmake \
  libegl1-mesa-dev libgles2-mesa-dev \
  libx11-dev libxfixes-dev libxcb1-dev libxau-dev libgl1-mesa-dev libxkbcommon-dev \
  libfontconfig1-dev libfreetype6-dev libharfbuzz-dev libgraphite2-dev \
  libc6-dev \
  libopenxr-loader1 libopenxr-dev libopenxr1-monado libwayland-dev \
  libjsoncpp-dev libdrm-dev libexpat1-dev libxcb-randr0-dev \
  libxml2-dev libffi-dev libbz2-dev libpng-dev libbrotli-dev liblzma-dev libglib2.0-dev libpcre2-dev
  </code></pre>
</details>

<details>
<summary>Arch Linux</summary> 
  <pre><code class="language-bash">
  sudo pacman -Syu --needed \
  base-devel \
  rust \
  cmake \
  mesa \
  libx11 \
  libxfixes \
  libxcb \
  libxkbcommon \
  fontconfig \
  freetype2 \
  openxr \
  jsoncpp \
  libffi \
  wayland \
  expat \
  libxml2 \
  libxau \
  bzip2 \
  xz \
  libpng \
  brotli \
  pcre2 \
  glib2 \
  libdrm
  </code></pre>
</details>
<details>
<summary>Fedora</summary> 
  <pre><code class="language-bash">
sudo dnf group install development-tools && \
sudo dnf install -y \
  cargo \
  cmake \
  mesa-libEGL-devel \
  mesa-libGLES-devel \
  libX11-devel \
  libXfixes-devel \
  libxcb-devel \
  libxkbcommon-devel \
  fontconfig-devel \
  freetype-devel \
  harfbuzz-devel \
  graphite2-devel \
  openxr-devel \ 
  wayland-devel \
  jsoncpp-devel \
  libdrm-devel \
  expat-devel \
  xcb-util-devel \
  libxml2-devel \
  libXau-devel \
  bzip2-devel \
  xz-devel \
  libpng-devel \
  brotli-devel \
  pcre2-devel \
  glib2-devel
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
