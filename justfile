alias b := build
alias i := install

root_dir := justfile_dir()
bin_dir := home_dir() / ".local" / "bin"

# Build all binaries, dynamically choosing between lld and ld
build:
    #!/usr/bin/env bash
    if command -v lld >/dev/null 2>&1; then
        echo "✅ lld found! Compiling with LLVM LLD Linker..."
        export RUSTFLAGS="-C link-arg=-fuse-ld=lld"
    else
        echo "⚠️  lld NOT found! Falling back to default system linker (ld)..."
        export RUSTFLAGS=""
    fi

    echo "Building mcp-synthesizer..."
    cargo build --release --manifest-path "{{ root_dir }}/Cargo.toml"

    echo -e "\n--- VERIFYING DEPENDENCIES VIA READELF ---"

    
# Install the binaries safely with md5sum checks
install: build
    #!/usr/bin/env bash
    echo -e "\nBEGINNING INSTALL..."
    mkdir -p "{{ bin_dir }}"

    # 1. Handle mcp_synth
    SRC_GDC="{{ root_dir }}/target/release/mcp_synth"
    DEST_GDC="{{ bin_dir }}/mcp_synth"

    if [ -f "$DEST_GDC" ] && [ "$(md5sum < "$SRC_GDC")" = "$(md5sum < "$DEST_GDC")" ]; then
        echo "mcp_synth is already up-to-date. Skipping."
    else
        cp "$SRC_GDC" "$DEST_GDC"
        echo "Installed/Updated mcp_synth"
    fi

    echo "INSTALL COMPLETE!"

# Start Redis via docker compose
redis-up:
    docker compose up -d

# Stop Redis
redis-down:
    docker compose down

# Run full tests with Redis (single-threaded to avoid FLUSHALL interference)
test: redis-up
    cargo test -- --test-threads 1
    docker compose down
