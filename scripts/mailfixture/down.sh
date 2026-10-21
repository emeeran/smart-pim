#!/usr/bin/env bash
docker rm -f pf-dovecot pf-mailpit >/dev/null 2>&1 || true
docker volume rm pf-maildata >/dev/null 2>&1 || true
