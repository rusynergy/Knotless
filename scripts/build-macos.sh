#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
case "$(uname -m)" in
  arm64) arch=arm64; archive=Xray-macos-arm64-v8a.zip ;;
  x86_64) arch=x64; archive=Xray-macos-64.zip ;;
  *) exit 2 ;;
esac
version=v26.10.31
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
base="https://github.com/XTLS/Xray-core/releases/download/$version/$archive"
curl -fsSL --retry 3 "$base" -o "$tmp/core.zip"
curl -fsSL --retry 3 "$base.dgst" -o "$tmp/core.dgst"
digest=$(shasum -a 256 "$tmp/core.zip" | awk '{print $1}')
grep -Eiq "(^|[^[:xdigit:]])$digest([^[:xdigit:]]|$)" "$tmp/core.dgst"
dest=src-tauri/resources/bundled-xray/current/xray
mkdir -p "$dest"
unzip -jo "$tmp/core.zip" xray LICENSE geoip.dat geosite.dat -d "$dest"
chmod +x "$dest/xray"
"$dest/xray" version
python3 - "$dest" "$arch" "$version" <<'PY'
import hashlib,json,pathlib,sys
d=pathlib.Path(sys.argv[1])
files={n:hashlib.sha256((d/n).read_bytes()).hexdigest() for n in ('xray','LICENSE','geoip.dat','geosite.dat')}
(d/'manifest.json').write_text(json.dumps(dict(version=sys.argv[3],architecture=sys.argv[2],files=files)))
PY
# Icon generation uses native macOS tools and the existing product artwork.
mkdir "$tmp/Knotless.iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" src-tauri/icons/icon.png --out "$tmp/Knotless.iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size*2))
  sips -z "$double" "$double" src-tauri/icons/icon.png --out "$tmp/Knotless.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$tmp/Knotless.iconset" -o src-tauri/icons/icon.icns
npm ci
npm test
npm run tauri -- build --bundles dmg
