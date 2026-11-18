# Wrap fish in lig unless lig itself started it; `fish` typed in another shell
# inside lig gets its own lig, like a tmux pane. `fish -ic 'cmd'` runs a command, not a session: left alone.
set -l __lig_cmd 0
set -l __lig_argv
if test -r /proc/$fish_pid/cmdline
    set __lig_argv (string split0 < /proc/$fish_pid/cmdline)
else if command -q ps
    set __lig_argv (string split ' ' -- (command ps -o args= -p $fish_pid))
end
for __lig_a in $__lig_argv[2..-1]
    string match -qr -- '^(-[A-Za-z]*c[A-Za-z]*|--command(=.*)?)$' $__lig_a; and set __lig_cmd 1
    string match -q -- '-*' $__lig_a; or break
    test $__lig_cmd = 1; and break
end
if status is-interactive; and test $__lig_cmd = 0; and test -z "$LIG_DISABLE"; and isatty stdin; and isatty stdout; and test "$TERM" != dumb; and test -z "$INSIDE_EMACS"; and test -x @LIG_BIN@
    # Parent pid: /proc on Linux (ps may be missing there), ps on macOS.
    set -l __lig_parent
    if test -r /proc/$fish_pid/stat
        set __lig_parent (string split ' ' -- (string replace -r '^.*\) ' '' < /proc/$fish_pid/stat))[2]
    else if command -q ps
        set __lig_parent (string trim -- (command ps -o ppid= -p $fish_pid))
    end
    # An unknown parent counts as lig: wrapping again would start lig in a loop.
    if test "$LIG_TTY" != (tty); or begin; test -n "$__lig_parent"; and test "$__lig_parent" != "$LIG_PID"; end
        # Ghostty's integration loaded here only; post.fish loads it in the shell inside lig.
        functions -q __ghostty_setup; and set -gx LIG_GHOSTTY 1
        if status is-login
            exec @LIG_BIN@ shell --shell (status fish-path) --login
        else
            exec @LIG_BIN@ shell --shell (status fish-path)
        end
    end
end
