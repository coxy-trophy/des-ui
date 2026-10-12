#!/bin/sh
# Publishes the packages in target/deb/ to the apt repository people install from,
# https://coxy-trophy.github.io/des-ui-apt, which GitHub Pages serves from the
# des-ui-apt repository. Build them first with packaging/build-deb.sh.
#
# The index is signed with the "DEs-UI packages" key in your GnuPG keyring, which
# never leaves this machine; DES_UI_KEY names another. The checkout of des-ui-apt is
# kept next to this one, or where DES_UI_APT says.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
key="${DES_UI_KEY:-DEs-UI packages}"
site="${DES_UI_APT:-$here/../des-ui-apt}"
url=https://coxy-trophy.github.io/des-ui-apt

ls "$here"/target/deb/*.deb >/dev/null 2>&1 || { echo "No packages yet: run packaging/build-deb.sh first." >&2; exit 1; }
gpg --list-secret-keys "$key" >/dev/null 2>&1 || { echo "No signing key called '$key' in your keyring." >&2; exit 1; }

[ -d "$site/.git" ] || git clone https://github.com/coxy-trophy/des-ui-apt.git "$site"
git -C "$site" pull -q --ff-only 2>/dev/null || true

"$here/packaging/make-repo.sh" "$site" "$key" >/dev/null
# Pages would otherwise run the files through Jekyll.
: >"$site/.nojekyll"
cat >"$site/README.md" <<TEXT
# DEs-UI packages

The apt repository for [DEs-UI](https://github.com/coxy-trophy/des-ui), a sci-fi
terminal desktop. For Ubuntu 26.04 on 64-bit Intel and AMD machines.

    curl -fsSL $url/des-ui.gpg | sudo tee /usr/share/keyrings/des-ui.gpg >/dev/null
    echo "deb [signed-by=/usr/share/keyrings/des-ui.gpg] $url ./" | sudo tee /etc/apt/sources.list.d/des-ui.list
    sudo apt update
    sudo apt install des-ui

Then log out and choose **DEs-UI** from the session menu on the login screen.
Updates arrive with the rest of the system's, through \`sudo apt upgrade\`.

The packages are signed with the key
\`$(gpg --list-keys --with-colons "$key" | sed -n 's/^fpr:*\([0-9A-F]*\):$/\1/p' | head -n 1)\`.
TEXT

git -C "$site" add -A
if git -C "$site" diff --cached --quiet; then
    echo "Nothing new to publish."
    exit 0
fi
git -C "$site" commit -q -m "Publish $(cd "$here/target/deb" && ls *.deb | tr '\n' ' ')"
git -C "$site" push -q origin HEAD
echo "Published. It is at $url within a minute or two."
