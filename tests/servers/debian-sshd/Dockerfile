# A small Debian sshd with three variants (VARIANT=kbd | oldkex | mosh). Test users only: test / test.
FROM debian:12-slim
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends openssh-server libpam-google-authenticator mosh locales ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && rm -f /etc/ssh/ssh_host_* \
 && mkdir -p /run/sshd \
 && useradd -m -s /bin/bash test && echo 'test:test' | chpasswd
COPY entry.sh /entry.sh
COPY variants /variants
RUN chmod +x /entry.sh
EXPOSE 22
ENTRYPOINT ["/entry.sh"]
