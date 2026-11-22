#!/usr/bin/env python3
"""Keeps the menu and settings icons of Memrium-Icons from answering for other icons' names.

KDE looks a name up in the active theme with `-parts` dropped from its end before it goes on to the inherited theme, so
`preferences-desktop` would also be drawn for preferences-desktop-screensaver, `computer` for computer-fail, and so on.
For each such name this puts a symlink <name>.svg in the theme to what KDE showed for it without the pack (Papirus).
A theme rebuild leaves them alone (it prunes only PNGs). Run again after adding names to PARENTS; it is idempotent.
"""
import os
import subprocess
import tempfile
from concurrent.futures import ThreadPoolExecutor

PARENTS = set('''bookmarks computer document-open-recent drive-harddisk drive-removable-media input-keyboard-virtual
preferences-desktop preferences-desktop-accessibility preferences-desktop-default-applications preferences-desktop-display
preferences-desktop-display-randr preferences-desktop-font preferences-desktop-keyboard preferences-desktop-locale
preferences-desktop-wallpaper preferences-security redshift-status-on system-run system-shutdown
tools-check-spelling'''.split())
# asked for by the menu itself: these are meant to get the new icon
INTENDED = {'computer-laptop'}
SIZES = {48: '22', 64: '64', 128: '128', 256: '256'}  # the 48 px folder is what panels and menus are given

ICONS = os.path.join(os.environ.get('XDG_DATA_HOME') or os.path.expanduser('~/.local/share'), 'icons')
SYSTEM = '/run/current-system/sw/share/icons'


def names(root, exts=('.png', '.svg', '.svgz')):
    out = set()
    for _, _, files in os.walk(os.path.realpath(root)):
        out |= {f.rsplit('.', 1)[0] for f in files if f.endswith(exts)}
    return out


def main():
    pack = names(os.path.join(ICONS, 'Memrium-Icons'), ('.png',))
    below = set()
    for root in (os.path.join(ICONS, 'Papirus'), os.path.join(ICONS, 'hicolor'), SYSTEM + '/breeze', SYSTEM + '/hicolor'):
        below |= names(root)

    def owner(name):
        while '-' in name:
            name = name.rsplit('-', 1)[0]
            if name in pack:
                return name
        return None

    # a name that got an icon of its own since: its guard would hide it (an SVG is preferred to a PNG)
    for theme in os.listdir(ICONS):
        if not theme.startswith('Memrium-Icons'):
            continue
        for size in SIZES:
            folder = os.path.join(ICONS, theme, f'{size}x{size}', 'apps')
            for file in os.listdir(folder) if os.path.isdir(folder) else ():
                name, ext = os.path.splitext(file)
                if ext == '.svg' and name in pack and os.path.islink(os.path.join(folder, file)):
                    os.remove(os.path.join(folder, file))

    todo = sorted(n for n in below - pack - INTENDED if owner(n) in PARENTS)
    with tempfile.TemporaryDirectory() as tmp:
        # an empty theme that inherits what the pack inherits: KDE picks another size of Papirus as the main theme
        # than as an inherited one
        empty = os.path.join(tmp, 'icons', 'Without-Pack')
        os.makedirs(os.path.join(empty, '48x48', 'apps'))
        with open(os.path.join(empty, 'index.theme'), 'w') as f:
            f.write('[Icon Theme]\nName=Without-Pack\nInherits=Papirus,hicolor\nDirectories=48x48/apps\n\n'
                    '[48x48/apps]\nContext=Applications\nSize=48\nType=Fixed\n')
        with open(os.path.join(tmp, 'kdeglobals'), 'w') as f:
            f.write('[Icons]\nTheme=Without-Pack\n')
        dirs = tmp + ':' + os.environ.get('XDG_DATA_DIRS', '/run/current-system/sw/share')
        env = dict(os.environ, XDG_CONFIG_HOME=tmp, XDG_CACHE_HOME=tmp, XDG_DATA_DIRS=dirs)

        def without_pack(job):
            name, size = job
            found = subprocess.run(['kiconfinder6', name, SIZES[size]], env=env, capture_output=True, text=True, timeout=20).stdout.strip()
            if found.startswith('/nix/store/') and '/share/icons/' in found:
                lasting = SYSTEM + '/' + found.split('/share/icons/', 1)[1]
                found = lasting if os.path.exists(lasting) else found
            return name, size, found

        with ThreadPoolExecutor(12) as pool:
            found = list(pool.map(without_pack, [(n, s) for n in todo for s in SIZES]))

    themes = sorted(t for t in os.listdir(ICONS) if t.startswith('Memrium-Icons') and os.path.isfile(os.path.join(ICONS, t, 'index.theme')))
    made, skipped = 0, set()
    for name, size, target in found:
        if not target.endswith('.svg'):
            skipped.add(name)
            continue
        for theme in themes:
            link = os.path.join(ICONS, theme, f'{size}x{size}', 'apps', name + '.svg')
            if os.path.islink(link):
                if os.readlink(link) == target:
                    continue
                os.remove(link)
            elif os.path.exists(link):
                continue  # a real file: someone's own icon
            os.symlink(target, link)
            made += 1
    print(f'{len(todo)} names guarded in {len(themes)} themes, {made} links written')
    if skipped:
        print('no SVG to point at:', ' '.join(sorted(skipped)))


if __name__ == '__main__':
    main()
