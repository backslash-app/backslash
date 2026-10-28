#!/bin/sh
# Sets the version everywhere it is written down: Cargo.toml, Cargo.lock,
# the AppStream release list and the AUR PKGBUILD (with the checksum of the
# tagged source when the tag already exists).
#   packaging/set-version.sh 0.4.0
set -eu
version=${1#v}
echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' || {
  echo "\"$version\" is not a version like 1.2.3" >&2
  exit 1
}
cd "$(dirname "$0")/.."

sed -i "/^\[workspace.package\]/,/^\[/ s/^version = \".*\"/version = \"$version\"/" Cargo.toml
awk -v v="$version" '
  /^name = "backslash(-core)?"$/ { print; getline; print "version = \"" v "\""; next }
  { print }
' Cargo.lock > Cargo.lock.new && mv Cargo.lock.new Cargo.lock

metainfo=data/app.backslash.Backslash.metainfo.xml
if ! grep -q "<release version=\"$version\"" "$metainfo"; then
  sed -i "s|^  <releases>|  <releases>\n    <release version=\"$version\" date=\"$(date -u +%F)\"/>|" "$metainfo"
fi

pkgbuild=packaging/aur/PKGBUILD
url="https://github.com/backslash-app/backslash/archive/refs/tags/v$version.tar.gz"
sum=$(curl -fsSL "$url" 2>/dev/null | sha256sum | cut -d' ' -f1) || sum=
[ "$sum" = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" ] && sum= # empty download
sed -i -e "s/^pkgver=.*/pkgver=$version/" -e "s/^pkgrel=.*/pkgrel=1/" \
  -e "s/^sha256sums=.*/sha256sums=('${sum:-SKIP}')/" "$pkgbuild"

echo "Version set to $version"
