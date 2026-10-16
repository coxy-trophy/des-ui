# DEs-UI shell integration for bash.
#
# Marks the prompt and each command's output with OSC 133, so that the terminal can
# show command blocks, jump between prompts, and copy a command's output; and tells
# it the working directory with OSC 7. Sourced after the user's own files.
# DES_UI_SHELL_INTEGRATION=0 turns it off.

[ "${DES_UI_SHELL_INTEGRATION-1}" = 0 ] && return 0
[ -n "$BASH_VERSION" ] || return 0
case "$-" in *i*) ;; *) return 0 ;; esac
[ -n "$__des_ui_integrated" ] && return 0
# Another integration is already marking prompts; two would confuse the terminal.
case "$PS1$PROMPT_COMMAND" in *'133;'*) return 0 ;; esac
__des_ui_integrated=1
__des_ui_at_prompt=0
__des_ui_running=0

# Percent-encodes $1 for a file: URL, into __des_ui_encoded, byte by byte.
__des_ui_url() {
    local LC_ALL=C c i hex
    __des_ui_encoded=
    for ((i = 0; i < ${#1}; i++)); do
        c=${1:i:1}
        case "$c" in
            [A-Za-z0-9_./~-]) __des_ui_encoded+=$c ;;
            *) printf -v hex '%%%02X' "'$c"; __des_ui_encoded+=$hex ;;
        esac
    done
}

# Runs first at every prompt: the command that just ran is over.
__des_ui_precmd() {
    local last=$?
    if [ "$__des_ui_running" = 1 ]; then
        printf '\033]133;D;%s\007' "$last"
        __des_ui_running=0
    fi
    __des_ui_url "$PWD"
    printf '\033]7;file://%s%s\007' "${HOSTNAME-}" "$__des_ui_encoded"
    return $last
}

# Runs last at every prompt, just before bash draws it.
__des_ui_ready() {
    __des_ui_at_prompt=1
    printf '\033]133;A\007'
}

# Runs before each command, through the DEBUG trap; only the first command of a
# line marks the start of its output.
__des_ui_preexec() {
    [ "$__des_ui_at_prompt" = 1 ] || return 0
    [ -n "$COMP_LINE" ] && return 0
    [ -n "$READLINE_LINE" ] && return 0
    case "$BASH_COMMAND" in __des_ui_*) return 0 ;; esac
    __des_ui_at_prompt=0
    __des_ui_running=1
    printf '\033]133;C\007'
}

if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == 'declare -a'* ]]; then
    PROMPT_COMMAND=(__des_ui_precmd "${PROMPT_COMMAND[@]}" __des_ui_ready)
else
    __des_ui_pc=${PROMPT_COMMAND-}
    while [[ $__des_ui_pc == *[\;[:space:]] ]]; do __des_ui_pc=${__des_ui_pc%?}; done
    PROMPT_COMMAND="__des_ui_precmd${__des_ui_pc:+; $__des_ui_pc}; __des_ui_ready"
    unset __des_ui_pc
fi
# A DEBUG trap of the user's own is left alone; then only the prompts are marked.
if [ -z "$(trap -p DEBUG)" ]; then
    trap '__des_ui_preexec' DEBUG
fi
# The end of the prompt, where the command is typed; invisible to readline.
if [ -n "$PS1" ]; then
    PS1="$PS1"'\[\033]133;B\007\]'
fi
