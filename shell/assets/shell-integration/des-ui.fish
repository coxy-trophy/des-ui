# DEs-UI shell integration for fish.
#
# Marks the prompt and each command's output with OSC 133, so that the terminal can
# show command blocks, jump between prompts, and copy a command's output; and tells
# it the working directory with OSC 7. Read from vendor_conf.d after the user's own
# configuration. DES_UI_SHELL_INTEGRATION=0 turns it off.

if status is-interactive
    and test "$DES_UI_SHELL_INTEGRATION" != 0
    and not set -q __des_ui_integrated
    set -g __des_ui_integrated 1
    set -g __des_ui_running 0

    # tmux consumes OSC unless our managed backend forwards it as passthrough.
    function __des_ui_osc
        if test "$DES_UI_PERSISTENT_TERMINAL" = 1; and test -n "$TMUX"
            printf '\033Ptmux;\033'
            printf '\033]'; printf $argv; printf '\007\033\\'
        else
            printf '\033]'; printf $argv; printf '\007'
        end
    end

    # Before each prompt: the command that just ran is over, and the prompt follows.
    function __des_ui_precmd --on-event fish_prompt
        set -l last $status
        if test "$__des_ui_running" = 1
            __des_ui_osc '133;D;%s' $last
            set -g __des_ui_running 0
        end
        __des_ui_osc '7;file://%s%s' "$hostname" (string escape --style=url -- "$PWD" | string replace -a '%2F' '/')
        __des_ui_osc '133;A'
    end

    # Before each command line runs.
    function __des_ui_preexec --on-event fish_preexec
        set -g __des_ui_running 1
        __des_ui_osc '133;C'
    end
end
