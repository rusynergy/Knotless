# Executed by macOS Authorization Services only for the lifetime of one connection.
set -eu
export PATH=/usr/bin:/bin:/usr/sbin:/sbin
umask 077
core=$1 config=$2 marker=$3 owner=$4 expected=$5 tun=$6
[ -f "$marker" ] || exit 1
work=$(mktemp -d /private/tmp/knotless-tun.XXXXXXXX)
pid='' service='' dns_changed=0
cleanup() {
    if [ "$dns_changed" = 1 ] && [ "$(networksetup -getdnsservers "$service")" = "172.31.255.1" ]; then
        if [ -s "$work/old-dns" ]; then
            set --
            while IFS= read -r ip; do set -- "$@" "$ip"; done < "$work/old-dns"
            networksetup -setdnsservers "$service" "$@" || :
        else
            networksetup -setdnsservers "$service" Empty || :
        fi
    fi
    if [ -n "$pid" ]; then
        kill -TERM "$pid" 2>/dev/null || :
        for i in 1 2 3 4 5; do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
        kill -KILL "$pid" 2>/dev/null || :
        wait "$pid" 2>/dev/null || :
    fi
    rm -f "$work/xray" "$work/config.json" "$work/geoip.dat" "$work/geosite.dat" "$work/old-dns" "$work/core.log"
    rmdir "$work"
}
trap cleanup EXIT
trap 'exit 1' INT HUP TERM
# Run only a private copy of the verified bundled executable.
cp "$core" "$work/xray"
[ "$(shasum -a 256 "$work/xray" | awk '{print $1}')" = "$expected" ] || { echo 'TUN requires the bundled Xray version' >&2; exit 1; }
chmod 700 "$work/xray"
cp "$config" "$work/config.json"
for asset in geoip.dat geosite.dat; do cp "$(dirname "$core")/$asset" "$work/$asset"; done
primary=$(printf 'show State:/Network/Global/IPv4\n' | scutil | awk '/PrimaryService :/ { print $3; exit }')
[ -n "$primary" ]
service=$(printf 'show Setup:/Network/Service/%s\n' "$primary" | scutil | sed -n 's/^ *UserDefinedName : //p' | head -1)
[ -n "$service" ]
old=$(networksetup -getdnsservers "$service")
case "$old" in
    "There aren't any DNS Servers set on "*) : > "$work/old-dns" ;;
    *) printf '%s\n' "$old" > "$work/old-dns"
       if grep -Ev '^[[:xdigit:].:%a-zA-Z0-9_-]+$' "$work/old-dns" >/dev/null; then exit 1; fi ;;
esac
cd "$work"
./xray run -c config.json > core.log 2>&1 &
pid=$!
i=0
while ! ifconfig "$tun" 2>/dev/null | grep -q '172.31.255'; do
    kill -0 "$pid" && kill -0 "$owner" && [ -f "$marker" ] && [ "$i" -lt 60 ] || exit 1
    sleep 0.25
    i=$((i+1))
done
dns_changed=1
networksetup -setdnsservers "$service" 172.31.255.1
while [ -f "$marker" ] && kill -0 "$owner" 2>/dev/null && kill -0 "$pid" 2>/dev/null; do sleep 0.5; done
