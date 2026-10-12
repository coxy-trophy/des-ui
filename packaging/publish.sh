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

The login screen and the boot splash are packages of their own:

    sudo apt install des-ui-greeter && sudo des-ui-login enable
    sudo apt install des-ui-plymouth

The packages are signed with the key
\`$(gpg --list-keys --with-colons "$key" | sed -n 's/^fpr:*\([0-9A-F]*\):$/\1/p' | head -n 1)\`.
TEXT

# The same words as a page: without one, the address itself answers "not found",
# though apt, which asks for the files by name, is served all the same.
{
    cat <<'HEAD'
<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>DEs-UI packages</title>
<style>
  body { background: #05080d; color: #aacfd1; font: 16px/1.6 ui-monospace, "DejaVu Sans Mono", monospace; max-width: 52rem; margin: 0 auto; padding: 3rem 1rem; }
  h1 { font-weight: normal; letter-spacing: .2em; text-transform: uppercase; }
  a { color: inherit; }
  pre { border: 1px solid #aacfd155; padding: 1rem; overflow-x: auto; }
  code { color: #e8fbfc; }
</style>
<h1>DEs-UI packages</h1>
HEAD
    sed -e '1,2d' -e 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g' \
        -e 's|\[DEs-UI\](\([^)]*\))|<a href="\1">DEs-UI</a>|' \
        -e 's|\*\*\([^*]*\)\*\*|<b>\1</b>|g' -e 's|`\([^`]*\)`|<code>\1</code>|g' "$site/README.md" \
        | awk 'BEGIN { RS = ""; FS = "\n" }
               /^    / { gsub(/(^|\n)    /, "\n"); sub(/^\n/, ""); print "<pre><code>" $0 "</code></pre>"; next }
               { print "<p>" $0 "</p>" }'
} >"$site/index.html"

git -C "$site" add -A
if git -C "$site" diff --cached --quiet; then
    echo "Nothing new to publish."
    exit 0
fi
git -C "$site" commit -q -m "Publish $(cd "$here/target/deb" && ls *.deb | tr '\n' ' ')"
git -C "$site" push -q origin HEAD
echo "Published. It is at $url within a minute or two."
