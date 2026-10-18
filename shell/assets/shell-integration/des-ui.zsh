# DEs-UI shell integration for zsh.
#
# Marks the prompt and each command's output with OSC 133, so that the terminal can
# show command blocks, jump between prompts, and copy a command's output; and tells
# it the working directory with OSC 7. Sourced after the user's own files.
# DES_UI_SHELL_INTEGRATION=0 turns it off.

[[ ${DES_UI_SHELL_INTEGRATION-1} == 0 ]] && return 0
[[ -o interactive ]] || return 0
(( ${+__des_ui_integrated} )) && return 0
# Another integration is already marking prompts; two would confuse the terminal.
[[ $PS1$PROMPT == *'133;'* ]] && return 0
typeset -g __des_ui_integrated=1 __des_ui_running=0 __des_ui_encoded=

# Only our private tmux backend needs passthrough; ordinary terminals keep OSC.
__des_ui_osc() {
    if [ "${DES_UI_PERSISTENT_TERMINAL-}" = 1 ] && [ -n "${TMUX-}" ]; then
        printf '\033Ptmux;\033'
        printf '\033]'; printf "$@"; printf '\007\033\\'
    else
        printf '\033]'; printf "$@"; printf '\007'
    fi
}

# Percent-encodes $1 for a file: URL, into __des_ui_encoded, byte by byte.
__des_ui_url() {
    local LC_ALL=C c hex
    __des_ui_encoded=
    for c in ${(s::)1}; do
        if [[ $c == [A-Za-z0-9_./~-] ]]; then
            __des_ui_encoded+=$c
        else
            printf -v hex '%%%02X' "'$c"
            __des_ui_encoded+=$hex
        fi
    done
}

# Before each prompt: the command that just ran is over, and the prompt follows.
__des_ui_precmd() {
    local last=$?
    if (( __des_ui_running )); then
        __des_ui_osc '133;D;%s' "$last"
        __des_ui_running=0
    fi
    __des_ui_url "$PWD"
    __des_ui_osc '7;file://%s%s' "${HOST-}" "$__des_ui_encoded"
    __des_ui_osc '133;A'
}

# Before each command line runs.
__des_ui_preexec() {
    __des_ui_running=1
    __des_ui_osc '133;C'
}

autoload -Uz add-zsh-hook
add-zsh-hook precmd __des_ui_precmd
add-zsh-hook preexec __des_ui_preexec
# The end of the prompt, where the command is typed; invisible to the line editor.
if [[ -n $PS1 ]]; then
    if [[ ${DES_UI_PERSISTENT_TERMINAL-} == 1 && -n ${TMUX-} ]]; then
        PS1="$PS1"$'%{\033Ptmux;\033\033]133;B\007\033\\%}'
    else
        PS1="$PS1"$'%{\033]133;B\007%}'
    fi
fi
