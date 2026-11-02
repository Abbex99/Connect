#!/bin/sh
set -eu
umask 077
cd "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
[ "$(uname -m)" = x86_64 ] || { echo 'Detta paket ar for Intel/AMD x86_64.' >&2; exit 1; }
[ "$(id -u)" = 0 ] || { echo 'Kor: sudo sh install.sh' >&2; exit 1; }
command -v docker >/dev/null || { echo 'Docker saknas. Installera CasaOS/Docker forst.' >&2; exit 1; }
docker compose version >/dev/null || { echo 'Docker Compose-plugin saknas.' >&2; exit 1; }
docker info >/dev/null
sha256sum -c SHA256SUMS
destination=/DATA/AppData/connect-server
state="$destination/state"
mkdir -p "$destination"
if [ -f "$destination/panel-host.txt" ]; then
    host=$(cat "$destination/panel-host.txt")
else
    host=${1:-}
    if [ -z "$host" ]; then
        printf 'CasaOS-datorns lokala IPv4-adress (exempel 192.168.1.50): '
        read -r host
    fi
fi
printf '%s\n' "$host" | awk -F. 'NF!=4 {exit 1} {for(i=1;i<=4;i++) if($i !~ /^[0-9]+$/ || length($i)>3 || (length($i)>1 && substr($i,1,1)=="0") || $i>255) exit 1; if($1==0 || $1>=224 || $1==127 || $4==0 || $4==255) exit 1}' || { echo 'Ange serverns lokala IPv4-adress, utan port eller https.' >&2; exit 1; }
version=$(cat VERSION)
case "$version" in *[!a-z0-9-]*|'') echo 'Ogiltig paketversion.' >&2; exit 1;; esac
docker build --tag "connect-server-local:$version" .
if [ ! -d "$state" ]; then
    mkdir "$state"
    chown 10001:10001 "$state"
    chmod 700 "$state"
fi
[ "$(stat -c %u "$state")" = 10001 ] || { echo 'Datamappen har fel agare. Ingen befintlig data har andrats.' >&2; exit 1; }
if [ ! -e "$state/operator.key" ]; then
    od -An -N32 -tx1 /dev/urandom | tr -d ' \n' > "$state/operator.key"
    printf '\n' >> "$state/operator.key"
    chown 10001:10001 "$state/operator.key"
    chmod 600 "$state/operator.key"
fi
# Keep previous configuration for recovery during upgrades; never remove state.
if [ -f "$destination/docker-compose.yml" ]; then
    cp "$destination/docker-compose.yml" "$destination/docker-compose.previous.yml"
fi
sed -e "s/@HOST@/$host/g" -e "s/@VERSION@/$version/g" docker-compose.template.yml > "$destination/docker-compose.yml"
cp Caddyfile "$destination/Caddyfile"
chmod 644 "$destination/Caddyfile"
mkdir -p "$destination/caddy-data" "$destination/caddy-config"
printf '%s\n' "$host" > "$destination/panel-host.txt"
docker compose -f "$destination/docker-compose.yml" config --quiet
docker compose -f "$destination/docker-compose.yml" up -d --force-recreate
printf '\nConnect Server ar installerad.\nPanel: https://%s:4750\n\nDin privata administrationsnyckel:\n' "$host"
cat "$state/operator.key"
printf '\nSpara nyckeln privat. Den ger full kontroll over servern.\nLas LAS-MIG.md om webblasaren visar en certifikatvarning.\n'
