#!/bin/sh
# Host keys are made at start, so `scripts/test-servers.sh rotate <name>` can replace them and restart.
set -e
ssh-keygen -A >/dev/null
cp "/variants/${VARIANT:?}/sshd_config" /etc/ssh/sshd_config
[ -f "/variants/$VARIANT/pam_sshd" ] && cp "/variants/$VARIANT/pam_sshd" /etc/pam.d/sshd
if [ "$VARIANT" = kbd ]; then
  # The code file must belong to the user, or the PAM module refuses it.
  cp /otp/secret /home/test/.google_authenticator
  chown test:test /home/test/.google_authenticator
  chmod 400 /home/test/.google_authenticator
fi
exec /usr/sbin/sshd -D -e
