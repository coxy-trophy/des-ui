#!/bin/sh
# Turns the packages in target/deb/ into an apt repository: a folder that, once it is
# served over HTTPS, people can add to their machine and install from with apt.
#
#     packaging/make-repo.sh <folder> [<gpg key id>]
#
# Packages already in the folder are kept, so that older versions stay installable.
# With a key the index is signed, which apt asks for; without one the repository only
# works for someone who marks it [trusted=yes], which is for trying it out.
#
# On the machine that installs, with the public key saved as des-ui.gpg:
#     sudo install -m644 des-ui.gpg /usr/share/keyrings/des-ui.gpg
#     echo "deb [signed-by=/usr/share/keyrings/des-ui.gpg] https://<where it is served>/ ./" \
#         | sudo tee /etc/apt/sources.list.d/des-ui.list
#     sudo apt update && sudo apt install des-ui
set -eu
[ $# -ge 1 ] || { sed -n '2,6p' "$0" | cut -c3- >&2; exit 2; }
packages=$(cd "$(dirname "$0")/../target/deb" && pwd)
mkdir -p "$1"
cd "$1"
cp "$packages"/*.deb .

dpkg-scanpackages --multiversion . >Packages 2>/dev/null
gzip -9 -k -f Packages

sums() {
    echo "$1:"
    for file in Packages Packages.gz; do
        printf ' %s %s %s\n' "$($2 "$file" | cut -d' ' -f1)" "$(wc -c <"$file")" "$file"
    done
}
{
    echo "Origin: DEs-UI"
    echo "Label: DEs-UI"
    echo "Architectures: $(dpkg --print-architecture)"
    echo "Date: $(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S UTC')"
    sums MD5Sum md5sum
    sums SHA256 sha256sum
} >Release

rm -f InRelease Release.gpg
if [ -n "${2:-}" ]; then
    gpg --batch --yes --local-user "$2" --clearsign -o InRelease Release
    gpg --batch --yes --local-user "$2" --detach-sign --armor -o Release.gpg Release
    gpg --batch --yes --export "$2" >des-ui.gpg
    echo "Signed with $2; the public key to hand out is $1/des-ui.gpg"
else
    echo "Not signed: installable only with [trusted=yes]."
fi
ls -1
