PREFIX := "usr"
BINARY := PREFIX / "bin"
DESTDIR := "/"

build:
    cargo build --release

test:
	cargo test

install:
    install -Dm755 target/release/stardust-xr-server {{ DESTDIR }}{{ BINARY }}/stardust-xr-server
