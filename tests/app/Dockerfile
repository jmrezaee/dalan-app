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

# The database tools the app runs for backups and restores (D12 to D18 hand checks K1 to K7): PostgreSQL 17's client
# (reads dumps from 13 to 17) and 16 (the test server's own: pg_restore 17 can't restore into 16), the MariaDB client (Debian's mysql, also for MySQL dumps) and MongoDB's tools.
RUN install -d /usr/share/postgresql-common/pgdg \
 && curl -fsSL -o /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc https://www.postgresql.org/media/keys/ACCC4CF8.asc \
 && echo "deb [signed-by=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc] https://apt.postgresql.org/pub/repos/apt bookworm-pgdg main" > /etc/apt/sources.list.d/pgdg.list \
 && apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends postgresql-client-17 postgresql-client-16 default-mysql-client \
 && ARCH="$(dpkg --print-architecture)" && case "$ARCH" in arm64) T=ubuntu2204-arm64;; *) T=debian12-x86_64;; esac \
 && curl -fsSL -o /tmp/mdt.deb "https://fastdl.mongodb.org/tools/db/mongodb-database-tools-$T-100.10.0.deb" \
 && apt-get install -y /tmp/mdt.deb && rm -f /tmp/mdt.deb \
 && rm -rf /var/lib/apt/lists/*

# pnpm itself, fetched now: the test network the runner joins has no way out to the registry.
RUN corepack prepare pnpm@10.30.0 --activate

WORKDIR /work

# The docker CLI, so a test can restart or rekey a server (the socket is mounted by run.mjs).
COPY --from=docker:cli /usr/local/bin/docker /usr/local/bin/docker
