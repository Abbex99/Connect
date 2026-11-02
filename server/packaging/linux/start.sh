#!/bin/sh
set -eu
docker compose -f /DATA/AppData/connect-server/docker-compose.yml up -d
printf 'Panel: https://%s:4750\n' "$(cat /DATA/AppData/connect-server/panel-host.txt)"
