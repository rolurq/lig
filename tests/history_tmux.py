#!/usr/bin/env python3
"""Ctrl+R shows atuin's matches, in zsh, bash, fish and nu, in a private tmux server. A fake
`atuin` on PATH stands in for the real one. Run after cargo build --release:
python3 tests/history_tmux.py."""
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path
from fish_pty import BIN

SHELLS = {
    'zsh': os.environ.get('LIG_TEST_ZSH') or shutil.which('zsh'),
    'bash': shutil.which('bash'),
    'fish': shutil.which('fish'),
    'nu': os.environ.get('LIG_TEST_NU') or shutil.which('nu'),
}
PROMPTS = {
    'zsh': "PROMPT='hist-test> '\n",
    'bash': "PS1='hist-test> '\n",
    'fish': "function fish_prompt; echo -n 'hist-test> '; end\nset -g fish_greeting\n",
    'nu': "$env.config.show_banner = false\n$env.PROMPT_COMMAND = {|| 'hist-test' }\n$env.PROMPT_COMMAND_RIGHT = ''\n",
}
# `atuin search … [-- query]`: the records containing the query, best last, NUL-terminated.
# Like atuin, it fails without $ATUIN_SESSION, which only the shell's own hook exports.
FAKE_ATUIN = r"""#!/bin/sh
[ -n "$ATUIN_SESSION" ] || { echo 'Failed to find $ATUIN_SESSION in the environment.' >&2; exit 1; }
printf '%s\n' "$*" >> "$FAKE_ATUIN_LOG"
[ -z "$FAKE_ATUIN_HANG" ] || sleep 30
q=
while [ $# -gt 0 ]; do
  if [ "$1" = -- ]; then q=$2; break; fi
  shift
done
for c in 'echo imported-by-atuin' 'echo hist-ok'; do
  case "$c" in *"$q"*) printf '5m\t%s\0' "$c" ;; esac
done
"""


@unittest.skipUnless(shutil.which('tmux'), 'tmux is required')
class HistoryTmux(unittest.TestCase):
    def start(self, kind, atuin=True, hang=False, config=''):
        shell = SHELLS[kind]
        if not shell:
            self.skipTest(f'{kind} is unavailable')
        self.dir = tempfile.TemporaryDirectory(prefix='lig-hist-')
        directory = Path(self.dir.name)
        home, runtime, bin_dir = directory / 'home', directory / 'runtime', directory / 'bin'
        home.mkdir()
        bin_dir.mkdir()
        runtime.mkdir(mode=0o700)
        # Ubuntu's /etc/zsh/zshrc runs compinit, which can stop to ask about folders' permissions.
        (home / '.zshenv').write_text('skip_global_compinit=1\n')
        self.state = directory / 'state'
        # A real atuin on this machine's PATH would answer instead of the fake (or the absence).
        path = os.pathsep.join(d for d in os.environ['PATH'].split(os.pathsep) if not (Path(d) / 'atuin').exists())
        if atuin:
            fake = bin_dir / 'atuin'
            fake.write_text(FAKE_ATUIN)
            fake.chmod(0o755)
            path = f'{bin_dir}{os.pathsep}{path}'
        self.atuin_log = directory / 'atuin.log'
        env = {'HOME': str(home), 'ZDOTDIR': str(home), 'XDG_CONFIG_HOME': str(home / '.config'),
               'XDG_DATA_HOME': str(home / '.local/share'), 'XDG_CACHE_HOME': str(home / '.cache'),
               'XDG_STATE_HOME': str(self.state), 'XDG_RUNTIME_DIR': str(runtime), 'SHELL': shell, 'LIG_LOG': '1',
               'HISTFILE': str(home / '.shell_history'), 'PATH': path, 'FAKE_ATUIN_LOG': str(self.atuin_log)}
        if hang:
            env['FAKE_ATUIN_HANG'] = '1'
        if config:
            (home / '.config/lig').mkdir(parents=True)
            (home / '.config/lig/config.toml').write_text(config)
        rc = {'zsh': home / '.zshrc', 'bash': home / '.bashrc', 'fish': home / '.config/fish/config.fish',
              'nu': home / '.config/nushell/config.nu'}[kind]
        rc.parent.mkdir(parents=True, exist_ok=True)
        rc.write_text(PROMPTS[kind])
        subprocess.run([BIN, 'install', kind], env=dict(os.environ, **env), check=True, capture_output=True)
        self.base = ['tmux', '-S', str(directory / 'socket'), '-f', '/dev/null']
        self.addCleanup(self.stop)
        launch = ['env', '-u', 'LIG_TTY', '-u', 'LIG_DISABLE', '-u', 'INSIDE_EMACS'] + [f'{k}={v}' for k, v in env.items()]
        self.tmux('new-session', '-d', '-x', '100', '-y', '30', '-s', 'test', '-c', str(home), *launch, shell)
        self.wait('no prompt', lambda s: 'hist-test' in s, timeout=15)

    def stop(self):
        subprocess.run(self.base + ['kill-server'], capture_output=True)
        # bash writes its history when the hangup reaches it, which can be mid-removal.
        for _ in range(50):
            try:
                return self.dir.cleanup()
            except OSError:
                time.sleep(.1)
        self.dir.cleanup()

    def tmux(self, *args):
        return subprocess.run(self.base + list(args), capture_output=True, text=True, timeout=5, check=True).stdout

    def screen(self):
        return self.tmux('capture-pane', '-p', '-t', 'test:0.0')

    def wait(self, what, check, timeout=6):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if check(self.screen()):
                return
            time.sleep(.1)
        log = self.state / 'lig/lig.log'
        self.fail(f'{what}; pane:\n{self.screen()}\nlig.log:\n{log.read_text()[-3000:] if log.exists() else ""}')

    def check_history(self, kind):
        self.start(kind)
        self.tmux('send-keys', '-t', 'test:0.0', 'C-r')
        self.wait('no history popup', lambda s: 'echo hist-ok' in s and 'imported-by-atuin' in s)
        self.assertIn('--exit=0 --exit=-1', self.atuin_log.read_text())
        self.tmux('send-keys', '-t', 'test:0.0', '-l', 'hist-o')
        self.wait('not narrowed', lambda s: 'imported-by-atuin' not in s and 'echo hist-ok' in s)
        self.tmux('send-keys', '-t', 'test:0.0', 'Enter')
        # The last prompt's line: nu may show a hint for the command under it.
        prompt = lambda s: [l for l in s.splitlines() if l.startswith('hist-test')][-1].rstrip()
        self.wait('not picked', lambda s: prompt(s).endswith('echo hist-ok') and '↺' not in s)

    def test_zsh(self):
        self.check_history('zsh')

    def test_bash(self):
        self.check_history('bash')

    def test_fish(self):
        self.check_history('fish')

    def test_nu(self):
        self.check_history('nu')

    def test_up_arrow_searches_the_current_folder(self):
        self.start('zsh')
        self.tmux('send-keys', '-t', 'test:0.0', 'Up')
        self.wait('no history popup', lambda s: 'echo hist-ok' in s and 'imported-by-atuin' in s)
        calls = self.atuin_log.read_text().splitlines()
        # The shell starts in the home folder; macOS may report it through /private.
        cwd = [c.split('--cwd ')[1].split(' ')[0] for c in calls if '--cwd ' in c]
        self.assertTrue(cwd and cwd[-1].endswith('/home'), calls)
        self.assertNotIn('--exit', calls[-1])
        self.tmux('send-keys', '-t', 'test:0.0', 'Escape')
        self.wait('popup stayed', lambda s: '↺' not in s)
        self.tmux('send-keys', '-t', 'test:0.0', 'C-r')
        self.wait('no global popup', lambda s: 'echo hist-ok' in s)
        last = self.atuin_log.read_text().splitlines()[-1]
        self.assertNotIn('--cwd', last)
        self.assertIn('--exit=0 --exit=-1', last)

    def test_a_hanging_atuin_does_not_stall_completion(self):
        self.start('zsh', hang=True, config='[behavior]\ngenerator_timeout_ms = 500\n')
        self.tmux('send-keys', '-t', 'test:0.0', 'C-r')
        self.wait('atuin was not asked', lambda s: self.atuin_log.exists() and self.atuin_log.read_text())
        # Once the search times out, lig falls back to the usual suggestions for the line.
        time.sleep(1.5)
        self.tmux('send-keys', '-t', 'test:0.0', '-l', 'ls -')
        # One worker answers history and completion: a stuck search would leave `ls -` without options.
        self.wait('completion stalled behind atuin', lambda s: '◆' in s, timeout=8)

    def test_without_atuin_ctrl_r_is_the_shells(self):
        self.start('bash', atuin=False)
        self.tmux('send-keys', '-t', 'test:0.0', 'C-r')
        self.wait("bash's own search did not open", lambda s: 'reverse-i-search' in s)


if __name__ == '__main__':
    unittest.main(verbosity=2)
