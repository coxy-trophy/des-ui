#!/bin/sh
# Builds the working copy and runs it in a window, on top of whatever desktop you are
# in (including an installed des-ui session). Nothing installed is touched, so this is
# the safe way to try a change. Arguments go to des-comp; see `./dev.sh --help`.
set -eu
cd "$(dirname "$0")"
cargo build
./target/debug/des-comp --write-cursor-theme ./target/cursor-theme/DEs-UI
export XCURSOR_THEME="${XCURSOR_THEME:-DEs-UI}"
export XCURSOR_SIZE="${XCURSOR_SIZE:-24}"
export XCURSOR_PATH="$PWD/target/cursor-theme:${XCURSOR_PATH:-$HOME/.icons:/usr/local/share/icons:/usr/share/icons:/usr/share/pixmaps}"
if [ "$XCURSOR_THEME" = DEs-UI ]; then
    case "${XDG_CACHE_HOME:-}" in
        /*) cursor_cache_root=$XDG_CACHE_HOME ;;
        *) cursor_cache_root=$HOME/.cache ;;
    esac
    export XCURSOR_PATH="$cursor_cache_root/des-ui/cursor-theme:$XCURSOR_PATH"
fi
exec ./target/debug/des-comp --backend winit "$@"
