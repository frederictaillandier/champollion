#!/bin/bash
# Installs or updates champollion-backend on the server, and adds WireGuard
# peers allowed to reach it. Safe to run again.
#
#   sudo ./setup-server.sh champollion-backend [NAME=PUBLIC_KEY@IP ...]
set -euo pipefail
cd "$(dirname "$0")"
binary=$1
shift

# Peers are added by editing wg0.conf and reloading it, so it must parse,
# and checked against the running interface's peers.
if ! wg-quick strip wg0 >/dev/null; then
    echo "fix /etc/wireguard/wg0.conf first" >&2
    exit 1
fi
if ! wg show wg0 >/dev/null; then
    echo "wg0 is down: systemctl start wg-quick@wg0" >&2
    exit 1
fi

apt-get install -y -q postgresql
id champollion >/dev/null 2>&1 ||
    useradd --system --no-create-home --shell /usr/sbin/nologin champollion
# The service connects through the local socket as the champollion user.
sudo -u postgres psql -tAc "SELECT 1 FROM pg_roles WHERE rolname = 'champollion'" | grep -q 1 ||
    sudo -u postgres createuser champollion
sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname = 'champollion'" | grep -q 1 ||
    sudo -u postgres createdb -O champollion champollion

for peer in "$@"; do
    name=${peer%%=*}
    rest=${peer#*=}
    key=${rest%@*}
    ip=${rest#*@}
    if wg show wg0 peers | grep -qxF "$key"; then
        echo "$name is already a WireGuard peer"
        continue
    fi
    if wg show wg0 allowed-ips | grep -qw "$ip/32"; then
        echo "$ip is already used by another WireGuard peer" >&2
        exit 1
    fi
    printf '\n# %s (champollion)\n[Peer]\nPublicKey = %s\nAllowedIPs = %s/32\n' \
        "$name" "$key" "$ip" >>/etc/wireguard/wg0.conf
    echo "added WireGuard peer $name at $ip"
done
wg syncconf wg0 <(wg-quick strip wg0)

ufw allow in on wg0 to 10.0.0.1 port 8090 proto tcp comment champollion-backend

install -m 755 "$binary" /usr/local/bin/champollion-backend
install -m 644 champollion-backend.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable champollion-backend
systemctl restart champollion-backend
sleep 2
systemctl --no-pager status champollion-backend | head -5
echo "health: $(curl -fsS http://10.0.0.1:8090/health || echo FAILED)"
echo "WireGuard server key: $(wg show wg0 public-key), port $(wg show wg0 listen-port)"
