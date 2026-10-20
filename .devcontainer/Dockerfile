FROM mcr.microsoft.com/devcontainers/rust:latest

# Runtime + build deps for eframe/winit (X11 + Wayland), wgpu (Vulkan/Mesa), and export (ffmpeg).
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
    && apt-get install -y --no-install-recommends \
        ffmpeg \
        mesa-vulkan-drivers libvulkan1 vulkan-tools libegl1 libgl1 \
        libxkbcommon-x11-0 libxkbcommon-dev \
        libwayland-client0 libwayland-cursor0 libwayland-egl1 libwayland-dev \
        libx11-6 libx11-xcb1 libxcursor1 libxrandr2 libxi6 libxcb1 \
        libfontconfig1-dev pkg-config \
        xclip \
    && rm -rf /var/lib/apt/lists/*

# English locales, so the host's LANG (passed in by devcontainer.json) works here
# too: Xenify Write picks its spelling (US/UK/AU…) from it.
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
    && apt-get install -y --no-install-recommends locales \
    && for l in en_US en_GB en_AU en_NZ en_CA en_IE en_IN en_ZA; do sed -i "s/^# *$l.UTF-8 UTF-8/$l.UTF-8 UTF-8/" /etc/locale.gen; done \
    && locale-gen && rm -rf /var/lib/apt/lists/*

# Browser build target (core crates must stay wasm-compatible).
RUN rustup target add wasm32-unknown-unknown

# Xenify Write and the Xenify desktop apps (Tauri): web view, packaging
# (Linux bundles, the Windows installer cross-built with MinGW + NSIS, Wine
# to try it), and headless testing (Xvfb, xdotool, ImageMagick, Chromium).
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
    && apt-get install -y --no-install-recommends \
        libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev librsvg2-dev libxdo-dev \
        libclang-dev libasound2-dev libpipewire-0.3-dev \
        gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 nsis wine wine64 \
        xvfb xdotool imagemagick chromium \
        brotli zstd file \
    && rm -rf /var/lib/apt/lists/*

# Test servers for Xenify Mail & Chat (mail, calendar, database).
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
    && apt-get install -y --no-install-recommends dovecot-imapd radicale postgresql \
    && rm -rf /var/lib/apt/lists/*

# Rust tools: Tauri's CLI (packages), wasm-bindgen (same version as the
# crates use), cargo-about (licence lists), twiggy (wasm sizes); the Windows
# target for the cross-built installer.
RUN rustup target add x86_64-pc-windows-gnu \
    && cargo install --locked tauri-cli --version 2.12.3 \
    && cargo install --locked wasm-bindgen-cli --version 0.2.128 \
    && cargo install --locked cargo-about --version 0.9.2 \
    && cargo install --locked twiggy --version 0.8.0 \
    && rm -rf /usr/local/cargo/registry /usr/local/cargo/git

# binaryen's wasm-opt, to shrink the browser build.
RUN curl -fsSL https://github.com/WebAssembly/binaryen/releases/download/version_124/binaryen-version_124-x86_64-linux.tar.gz \
        | tar xz -C /opt \
    && ln -s /opt/binaryen-version_124/bin/wasm-opt /usr/local/bin/wasm-opt
