#!/usr/bin/env python3
"""Read-only owner notifications; no refresh commands, jobs or config writes."""
import ctypes
import json
import os
from pathlib import Path
import selectors
import socket
import struct
import sys


def main():
    libc = ctypes.CDLL(None, use_errno=True)
    fd = libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
    if fd < 0:
        raise OSError(ctypes.get_errno(), 'Cannot watch Settings owners')
    selector = selectors.DefaultSelector()
    selector.register(fd, selectors.EVENT_READ, 'files')
    watches = {}
    # Watch directories, not replaced file inodes. Keep roots watched so newly
    # installed/removed skills and plugins are noticed as well as file edits.
    home = Path.home()
    roots = [(home / '.config/hypr', 'devices'), (home / '.config/omarchy', 'plugins'),
             (home / '.agents/skills', 'skills'), (home / '.codex/skills', 'skills'),
             (home / '.claude/skills', 'skills'), (home / '.cursor/skills', 'skills'),
             (home / '.config/opencode/skills', 'skills'), (home / '.pi/agent/skills', 'skills'),
             (home / '.gemini/config/plugins', 'skills'), (home / '.gemini/antigravity/builtin/skills', 'skills'),
             (home / '.gemini/antigravity-cli/builtin/skills', 'skills'), (home / '.codex', 'plugins')]
    mask = 0x00000008 | 0x00000080 | 0x00000100 | 0x00000200 | 0x00000400 | 0x00000040

    def discover():
        known = {path for path, domain in watches.values()}
        for root, domain in roots:
            paths = [root]
            if root.is_dir() and root != home / '.codex':
                for directory, children, files in os.walk(root, followlinks=False):
                    children[:] = [name for name in children if name not in ('node_modules', '.git', 'target', '.venv', '__pycache__') and not (Path(directory) / name).is_symlink()]
                    paths.extend(Path(directory) / name for name in children)
            for path in paths:
                if path in known or not path.is_dir():
                    continue
                wd = libc.inotify_add_watch(fd, os.fsencode(path), mask)
                if wd >= 0:
                    watches[wd] = (path, domain)
                    known.add(path)

    discover()
    session = os.environ.get('HYPRLAND_INSTANCE_SIGNATURE', '')
    socket_path = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/' + str(os.getuid()))) / 'hypr' / session / '.socket2.sock'
    compositor = None
    pending = b''
    while True:
        if compositor is None and session:
            try:
                compositor = socket.socket(socket.AF_UNIX)
                compositor.connect(str(socket_path))
                compositor.setblocking(False)
                selector.register(compositor, selectors.EVENT_READ, 'compositor')
            except OSError:
                if compositor:
                    compositor.close()
                compositor = None
        changed = set()
        for key, _ in selector.select(2 if compositor is None else None):
            if key.data == 'files':
                data = os.read(fd, 65536)
                offset = 0
                while offset + 16 <= len(data):
                    wd, event_mask, cookie, size = struct.unpack_from('iIII', data, offset)
                    offset += 16 + size
                    if wd in watches:
                        changed.add(watches[wd][1])
                    if event_mask & 0x8000:
                        watches.pop(wd, None)
                discover()
            else:
                data = compositor.recv(65536)
                if not data:
                    selector.unregister(compositor)
                    compositor.close()
                    compositor = None
                    pending = b''
                    changed.add('devices')
                else:
                    pending += data
                    while b'\n' in pending:
                        line, pending = pending.split(b'\n', 1)
                        event = line.partition(b'>>')[0]
                        if event.startswith((b'monitor', b'workspace', b'moveworkspace', b'createworkspace', b'destroyworkspace')) or event in (b'configreloaded', b'activelayout'):
                            changed.add('devices')
        if changed:
            print(json.dumps(sorted(changed)), flush=True)


if __name__ == '__main__':
    try:
        main()
    except BrokenPipeError:
        pass
