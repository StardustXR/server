name := 'stardust-xr-server'

# Use mold linker if clang and mold exists.

clang-path := `which clang || true`
mold-path := `which mold || true`
linker-arg := if clang-path != '' { if mold-path != '' { '-C linker=' + clang-path + ' -C link-arg=--ld-path=' + mold-path + ' ' } else { '' } } else { '' }
export RUSTFLAGS := linker-arg + env('RUSTFLAGS', '')
rootdir := ''
prefix := '/usr'
base-dir := absolute_path(clean(rootdir / prefix))
export INSTALL_DIR := base-dir / 'share'
cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
bin-src := cargo-target-dir / 'release' / name
bin-dst := base-dir / 'bin' / name

# Default recipe which runs `just build-release`
default: build-release

# Runs `cargo clean`
clean:
    cargo clean

# `cargo clean` and removes vendored dependencies
clean-dist: clean
    rm -rf .cargo vendor vendor.tar

# Compiles with debug profile
build-debug *args:
    cargo build {{ args }}

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Compiles release profile with vendored dependencies
build-vendored *args: vendor-extract (build-release '--frozen --offline' args)

# Runs a clippy check
check *args:
    cargo clippy --all-features {{ args }} -- -W clippy::pedantic

# Runs a clippy check with JSON message format
check-json: (check '--message-format=json')

# Run with debug logs
run *args:
    env RUST_LOG=debug RUST_BACKTRACE=1 cargo run --release {{ args }}

# Installs files
install:
    install -Dm0755 {{ bin-src }} {{ bin-dst }}

# Uninstalls installed files
uninstall:
    rm {{ bin-dst }}

# Vendor dependencies locally
vendor:
    mkdir -p .cargo
    cargo vendor --sync Cargo.toml --sync config/Cargo.toml | head -n -1 > .cargo/config.toml
    echo 'directory = "vendor"' >> .cargo/config.toml
    tar pcf vendor.tar vendor
    rm -rf vendor

# Extracts vendored dependencies
vendor-extract:
    #!/usr/bin/env sh
    rm -rf vendor
    tar pxf vendor.tar
