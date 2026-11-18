# lig: wrap this interactive bash in the lig PTY (once per terminal). `bash -ic 'cmd'` (`c` in $-) runs a command, not a session: left alone.
if [[ $- == *i* && $- != *c* && -z "$LIG_DISABLE" && -t 0 && -t 1 && "$TERM" != dumb && -z "$INSIDE_EMACS" \
      && -x "@LIG_BIN@" && "$LIG_TTY" != "$(tty)" ]]; then
  # Ghostty's integration sources this file; post.bash loads it in the shell inside lig.
  [[ " ${BASH_SOURCE[*]}" == *"/shell-integration/bash/ghostty.bash"* ]] && export LIG_GHOSTTY=1
  if shopt -q login_shell; then
    exec "@LIG_BIN@" shell --shell "$BASH" --login
  else
    exec "@LIG_BIN@" shell --shell "$BASH"
  fi
fi
