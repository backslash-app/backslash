#!/bin/sh
# Publishes the AUR package `backslash` for a released version. Runs as root in
# an Arch Linux container (see .github/workflows/release.yml).
#   AUR_SSH_PRIVATE_KEY=… packaging/aur/publish.sh 0.4.0
# Without AUR_SSH_PRIVATE_KEY, prepares the package and stops (a dry run).
set -eu
version=${1#v}
here=$(cd "$(dirname "$0")" && pwd)
url="https://github.com/backslash-app/backslash/archive/refs/tags/v$version.tar.gz"
sum=$(curl -fsSL "$url" | sha256sum | cut -d' ' -f1)

work=$(mktemp -d)
sed -e "s/^pkgver=.*/pkgver=$version/" -e "s/^pkgrel=.*/pkgrel=1/" \
  -e "s/^sha256sums=.*/sha256sums=('$sum')/" "$here/PKGBUILD" > "$work/PKGBUILD"
# makepkg refuses to run as root.
id builder >/dev/null 2>&1 || useradd -m builder
chown -R builder "$work"
su builder -c "cd '$work' && makepkg --printsrcinfo > .SRCINFO"
cat "$work/.SRCINFO"

if [ -z "${AUR_SSH_PRIVATE_KEY:-}" ]; then
  echo "::notice::No AUR_SSH_PRIVATE_KEY secret: the AUR package was prepared but not published"
  exit 0
fi
mkdir -p ~/.ssh
chmod 700 ~/.ssh
printf '%s\n' "$AUR_SSH_PRIVATE_KEY" > ~/.ssh/aur
chmod 600 ~/.ssh/aur
ssh-keyscan -t ed25519 aur.archlinux.org >> ~/.ssh/known_hosts 2>/dev/null
export GIT_SSH_COMMAND="ssh -i $HOME/.ssh/aur -o IdentitiesOnly=yes"

git clone -q ssh://aur@aur.archlinux.org/backslash.git "$work/aur"
cp "$work/PKGBUILD" "$work/.SRCINFO" "$work/aur/"
cd "$work/aur"
git add PKGBUILD .SRCINFO
if git diff --cached --quiet; then
  echo "The AUR already has backslash $version"
  exit 0
fi
git -c user.name="Steven Persia" -c user.email="persia.steven8@gmail.com" commit -qm "Update to $version"
git push -q origin HEAD:master
echo "Published backslash $version to the AUR"
