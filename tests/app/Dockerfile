# The real Dalan on Linux: WebKitGTK, tauri-driver (WebDriver for Tauri), WebdriverIO's Node, a virtual screen and a
# recorder. tauri-driver supports Linux and Windows only, so this is how the real binary is driven from a Mac.
FROM rust:1-bookworm

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      curl ca-certificates gnupg build-essential pkg-config git \
      libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libsoup-3.0-dev \
      webkit2gtk-driver xvfb xauth dbus-x11 ffmpeg openssh-client fonts-dejavu-core \
 && curl -fsSL https://deb.nodesource.com/setup_24.x | bash - \
 && apt-get install -y --no-install-recommends nodejs \
 && corepack enable \
 && rm -rf /var/lib/apt/lists/*

RUN cargo install tauri-driver --locked

WORKDIR /work

# The docker CLI, so a test can restart or rekey a server (the socket is mounted by run.mjs).
COPY --from=docker:cli /usr/local/bin/docker /usr/local/bin/docker
