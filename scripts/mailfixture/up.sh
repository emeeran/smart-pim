#!/usr/bin/env bash
# Test mail fixtures: Dovecot (IMAP 10143 / POP3 10110) + Mailpit SMTP sink (1025).
# User: test@example.com / secret123  (see ./users)
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
docker rm -f pf-dovecot pf-mailpit >/dev/null 2>&1 || true
docker run -d --name pf-dovecot -p 10143:10143 -p 10110:10110 \
  -v "$DIR/dovecot.conf:/etc/dovecot/dovecot.conf:ro" \
  -v "$DIR/users:/etc/dovecot/users:ro" \
  -v pf-maildata:/srv/vmail dovecot/dovecot:latest >/dev/null
docker run -d --name pf-mailpit -p 1025:1025 -p 8025:8025 axllent/mailpit >/dev/null
for i in $(seq 1 30); do
  if (timeout 1 bash -c 'exec 3<>/dev/tcp/127.0.0.1/10143 && head -c 3 <&3 | grep -q "OK"') 2>/dev/null \
     && (timeout 1 bash -c 'exec 3<>/dev/tcp/127.0.0.1/1025 && head -c 3 <&3 | grep -q "220"') 2>/dev/null; then
    echo "fixtures ready"; exit 0
  fi
  sleep 1
done
echo "fixtures failed to start"; docker logs pf-dovecot | tail -20; exit 1
