#!/bin/sh
set -eu
docker compose -f /DATA/AppData/connect-server/docker-compose.yml stop
echo 'Servern ar stoppad. Data och nycklar finns kvar.'
