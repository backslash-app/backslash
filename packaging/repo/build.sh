#!/bin/sh
# Builds the apt and dnf repositories of a release, signed with the
# repository key, ready to publish as a static site (GitHub Pages).
#   REPO_SIGNING_KEY="$(cat key.asc)" packaging/repo/build.sh dist/ site/ https://backslash-app.github.io/backslash
# dist/ holds the release's .deb and .rpm files. REPO_SIGNING_KEY is an
# ASCII-armored GPG private key without a passphrase (see CONTRIBUTING.md).
set -eu
dist=$(cd "$1" && pwd)
mkdir -p "$2"
out=$(cd "$2" && pwd)
url=${3%/}

if [ -z "${REPO_SIGNING_KEY:-}" ]; then
  echo "::notice::No REPO_SIGNING_KEY secret: the apt and dnf repositories were not built"
  exit 0
fi
GNUPGHOME=$(mktemp -d)
export GNUPGHOME
trap 'rm -rf "$GNUPGHOME"' EXIT
printf '%s\n' "$REPO_SIGNING_KEY" | gpg --batch --quiet --import
key=$(gpg --list-secret-keys --with-colons | awk -F: '/^fpr/ { print $10; exit }')
gpg --armor --export "$key" > "$out/key.asc"

# apt: one "stable" suite, signed (InRelease and Release.gpg).
apt=$out/apt
mkdir -p "$apt/pool/main"
cp "$dist"/*.deb "$apt/pool/main/"
cd "$apt"
for arch in amd64 arm64; do
  mkdir -p "dists/stable/main/binary-$arch"
  apt-ftparchive --arch "$arch" packages pool > "dists/stable/main/binary-$arch/Packages"
  gzip -9kf "dists/stable/main/binary-$arch/Packages"
done
apt-ftparchive \
  -o APT::FTPArchive::Release::Origin=Backslash \
  -o APT::FTPArchive::Release::Label=Backslash \
  -o APT::FTPArchive::Release::Suite=stable \
  -o APT::FTPArchive::Release::Codename=stable \
  -o "APT::FTPArchive::Release::Architectures=amd64 arm64" \
  -o APT::FTPArchive::Release::Components=main \
  release dists/stable > Release.tmp
mv Release.tmp dists/stable/Release
gpg --batch --yes -u "$key" --clearsign -o dists/stable/InRelease dists/stable/Release
gpg --batch --yes -u "$key" --armor --detach-sign -o dists/stable/Release.gpg dists/stable/Release

# dnf: signed packages and signed metadata.
rpm=$out/rpm
mkdir -p "$rpm"
cp "$dist"/*.rpm "$rpm/"
rpmsign --define "_gpg_name $key" --define "_gpg_path $GNUPGHOME" --addsign "$rpm"/*.rpm > /dev/null
createrepo_c --quiet "$rpm"
gpg --batch --yes -u "$key" --armor --detach-sign "$rpm/repodata/repomd.xml"
cat > "$rpm/backslash.repo" <<EOF
[backslash]
name=Backslash
baseurl=$url/rpm
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=$url/key.asc
EOF

cat > "$out/index.html" <<EOF
<!doctype html>
<meta charset="utf-8">
<title>Backslash packages</title>
<h1>Backslash packages</h1>
<p>Debian, Ubuntu and derivatives:</p>
<pre>curl -fsSL $url/key.asc | sudo gpg --dearmor -o /etc/apt/keyrings/backslash.gpg
echo "deb [signed-by=/etc/apt/keyrings/backslash.gpg] $url/apt stable main" | sudo tee /etc/apt/sources.list.d/backslash.list
sudo apt update &amp;&amp; sudo apt install backslash</pre>
<p>Fedora and derivatives:</p>
<pre>sudo dnf config-manager addrepo --from-repofile=$url/rpm/backslash.repo
sudo dnf install backslash</pre>
<p><a href="https://github.com/backslash-app/backslash">github.com/backslash-app/backslash</a></p>
EOF
echo "Repositories built in $out"
