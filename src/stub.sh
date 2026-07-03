# trapdoor shell stub
#
# Injected into the target script through BASH_ENV. Pure bash, zero
# dependencies: it phones home to the controller over /dev/tcp (a bash
# builtin), then arms the DEBUG trap so every simple command asks the
# controller for permission before running.

[[ -n ${TRAPDOOR_PORT:-} && -z ${__TD_ACTIVE:-} ]] || return 0
if ! exec {__TD_FD}<>"/dev/tcp/127.0.0.1/${TRAPDOOR_PORT}"; then
    return 0
fi
export __TD_ACTIVE=1

__td_stop() {
    # Pipelines and $(...) run in subshells that inherit this trap; a second
    # writer on the shared socket would corrupt the protocol, so stay quiet.
    (( BASH_SUBSHELL == 0 )) || return 0

    local __td_line=$1 __td_src=$2 __td_cmd=$3
    # FUNCNAME includes __td_stop itself; report the script's own depth.
    local __td_depth=$(( ${#FUNCNAME[@]} - 1 ))
    __td_cmd=${__td_cmd//$'\n'/ }
    __td_cmd=${__td_cmd//$'\t'/ }
    printf 'STOP\t%s\t%s\t%s\t%s\n' \
        "$__td_src" "$__td_line" "$__td_depth" "$__td_cmd" >&"${__TD_FD}"

    local __td_reply
    while IFS= read -r __td_reply <&"${__TD_FD}"; do
        case ${__td_reply} in
            GO)
                return 0 ;;
            KILL)
                exit 127 ;;
            "EVAL "*)
                # Runs in the *live* shell: assignments stick, functions are
                # visible, the script's state is yours to inspect or mutate.
                { eval "${__td_reply#EVAL }"; } >&"${__TD_FD}" 2>&1
                printf '\n\x04END %s\n' "$?" >&"${__TD_FD}"
                ;;
            BT)
                local __td_i
                for (( __td_i = 1; __td_i < ${#FUNCNAME[@]}; __td_i++ )); do
                    printf '#%d  %s()  at %s:%s\n' "$(( __td_i - 1 ))" \
                        "${FUNCNAME[__td_i]}" \
                        "${BASH_SOURCE[__td_i]:-?}" \
                        "${BASH_LINENO[__td_i - 1]:-?}" >&"${__TD_FD}"
                done
                printf '\n\x04END 0\n' >&"${__TD_FD}"
                ;;
        esac
    done

    # Controller vanished: disarm and let the script run free.
    trap - DEBUG
    return 0
}

# Note: functrace (not extdebug) — enabling extdebug inside a startup file
# makes bash try to load the bashdb profile, which we neither need nor want.
set -o functrace
trap '__td_stop "$LINENO" "${BASH_SOURCE[0]:-main}" "$BASH_COMMAND"' DEBUG
