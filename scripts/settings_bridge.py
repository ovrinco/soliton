#!/usr/bin/env python3
"""Settings owner adapter for plugins and skills. JSONL requests; no work or writes on startup.

The desktop's monitors and bindings are read and changed by the engine (`desktop-*` commands)."""
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

sys.dont_write_bytecode = True


def run(argv, **kwargs):
    result = subprocess.run(argv, capture_output=True, text=True, timeout=12, **kwargs)
    if result.returncode:
        raise ValueError((result.stderr or result.stdout).strip()[:4000] or 'Owner command failed')
    return result.stdout


class Settings:
    def __init__(self):
        self.plugin_owner = None

    def plugin_module(self):
        if self.plugin_owner is None:
            path = Path.home() / '.config/omarchy/plugins/ian.plugin-manager/backend.py'
            if not path.is_file():
                raise ValueError('The bar\'s plugin manager is not installed')
            spec = importlib.util.spec_from_file_location('soliton_plugin_owner', path)
            self.plugin_owner = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(self.plugin_owner)
            if os.environ.get('SOLITON_PLUGINS_DRY') == '1':
                self.plugin_owner.ipc = self.dry_ipc(self.plugin_owner.ipc)
        return self.plugin_owner

    def dry_ipc(self, real):
        # Dry runs read the bar for real, never write it: each write is logged
        # as its exact command and answered as the bar answers, then overlaid
        # on later reads so the page redraws from that answer.
        enabled, layout_moves = {}, []
        log = Path(os.environ.get('SOLITON_PLUGINS_DRY_LOG') or Path(os.environ['SOLITON_DATA']) / 'plugin_commands.log')

        def ipc(method, *args):
            if method in ('enablePlugin', 'setPluginEnabled', 'moveBarWidget'):
                command = ['omarchy-shell', 'shell', method, *args]
                with open(log, 'a') as stream:
                    stream.write(time.strftime('%Y-%m-%dT%H:%M:%S') + ' ' + ' '.join(
                        __import__('shlex').quote(part) for part in command) + '\n')
                if method == 'moveBarWidget':
                    layout_moves.append((args[0], json.loads(args[1])))
                else:
                    enabled[args[0]] = method == 'enablePlugin' or args[1] == 'true'
                return 'ok'
            output = real(method, *args)
            if method == 'listPlugins':
                rows = json.loads(output)
                for row in rows:
                    if row.get('id') in enabled:
                        row['enabled'] = enabled[row['id']]
                return json.dumps(rows)
            if method == 'listShellConfig':
                config = json.loads(output)
                layout = config.setdefault('bar', {}).setdefault('layout', {})
                for plugin_id, placement in layout_moves:
                    entry = plugin_id
                    for section in ('left', 'center', 'right'):
                        entries = layout.get(section, [])
                        for item in list(entries):
                            if (item.get('id') if isinstance(item, dict) else item) == plugin_id:
                                entry = item
                                entries.remove(item)
                    target = layout.setdefault(placement['section'], [])
                    index = placement.get('index', len(target))
                    target.insert(min(index, len(target)), entry)
                return json.dumps(config)
            return output
        return ipc

    def plugins(self, request):
        owner = self.plugin_module()
        # The bar's manager owns the verbs, their protection rules and refusals.
        result = owner.handle(request)
        known = {row['id'] for row in result['plugins']}
        kind_words = {'bar-widget': 'Bar widget', 'panel': 'Panel', 'service': 'Service', 'launcher': 'Launcher'}
        rows = [dict(id=row['id'], name=row.get('name') or row['id'], purpose=(row.get('description') or '').split('\n')[0],
                     description=' '.join((row.get('description') or '').split()),
                     enabled=row.get('enabled'), installed=True, protected=bool(row.get('protected')),
                     firstParty=bool(row.get('firstParty')),
                     kinds=[kind_words.get(kind, kind.replace('-', ' ').capitalize()) for kind in (row.get('kinds') or [])],
                     widget='bar-widget' in (row.get('kinds') or []), section=row.get('section') or '',
                     position=row.get('position', -1), sectionSize=row.get('sectionSize', 0))
                for row in result['plugins']]
        try:
            catalog = json.loads(run(['omarchy-plugin-catalog']))
        except Exception:
            catalog = []
        for meta in catalog:
            if meta.get('id') not in known:
                rows.append(dict(id=meta['id'], name=meta.get('name') or meta['id'], purpose=meta.get('description', ''),
                                 description=' '.join(meta.get('description', '').split()), firstParty=False, kinds=[],
                                 enabled=None, installed=False, protected=True, widget=False, section='',
                                 position=-1, sectionSize=0))
        rows.sort(key=lambda row: row['name'].lower())
        return {'plugins': rows, 'message': result.get('message', '')}

    # Skills: the skill manager's own engine (omarchy_skills), imported
    # read-only from its plugin folder. The page receives names, never paths:
    # each skill is addressed by the engine's id, and the bridge keeps the
    # id-to-path map for the requests that need a path.
    SKILL_PLUGIN = Path.home() / '.config/omarchy/plugins/firstpick.skill-manager'

    def skill_engine(self):
        if getattr(self, 'skill_bridge', None) is None:
            if not (self.SKILL_PLUGIN / 'omarchy_skills/gui.py').is_file():
                raise ValueError('The skill manager is not installed')
            sys.path.insert(0, str(self.SKILL_PLUGIN))
            from omarchy_skills import common, gui, policy, skills
            self.skill_modules = dict(common=common, gui=gui, policy=policy, skills=skills)
            home = os.environ.get('SOLITON_HARNESS_HOME')
            if home:
                home = Path(home).resolve()
                context = common.Context(home, home / '.config', home / '.local/share', None,
                                         home / '.pi/agent', home / '.codex')
            else:
                context = common.Context.current()
            self.skill_bridge = gui.Bridge(context)
            self.skill_paths = {}
            self.skill_last = None
        return self.skill_bridge

    def skills_home(self):
        return Path(os.environ.get('SOLITON_SKILLS_HOME') or Path.home() / '01_governance/skills')

    def skill_label(self, tool):
        return self.skill_modules['common'].LABELS.get(tool, tool)

    def skill_collection(self, ctx, location):
        # A location named by its collection, never by its folder.
        path, source = Path(location['path']), location.get('source', '')
        home = self.skills_home()
        if path.is_relative_to(home):
            return 'Skills home'
        if source.startswith('Codex plugin '):
            return 'Codex plugin · ' + source[len('Codex plugin '):].split('@')[0]
        if path.is_relative_to(ctx.home / '.agents/skills'):
            return 'Shared agents folder'
        if path.is_relative_to(ctx.codex / 'skills'):
            return 'Codex system folder'
        for tool in self.skill_modules['common'].TOOLS:
            if path.is_relative_to(ctx.target(tool)):
                return self.skill_label(tool) + ' folder'
        return 'Pi package' if 'Pi' in source else 'Local collection'

    def skill_scrub(self, text):
        # Engine words name folders; the page names skills and tools instead.
        text = str(text)
        for tool, label in self.skill_modules['common'].LABELS.items():
            text = re.sub(r'\b' + tool + r'/', label + ' · ', text)
        text = re.sub(r'(?:~|/)[^\s:;,]*/([^\s/:;,]+)', r'\1', text)
        text = re.sub(r'\b(?:on|in|for) (?:pi|opencode|claude|cursor|codex)\b',
                      lambda m: m.group(0).split()[0] + ' ' + self.skill_label(m.group(0).split()[1]), text)
        return text.replace('Controls > Preview recovery', 'Restrictions, Preview recovery')

    def skill_links(self, ctx, rows):
        # Linked to: an entry in a tool's own skills folder that is a link
        # resolving to this skill's original folder (managed or not).
        tools = self.skill_modules['common'].TOOLS
        by_dir = {}
        for row in rows:
            try:
                by_dir.setdefault(Path(row['path']).parent.resolve(strict=True), []).append(row)
            except OSError:
                pass
        for row in rows:
            row['linked'] = set(row.get('linked_targets') or [])
        for tool in tools:
            folder = ctx.target(tool)
            if not folder.is_dir():
                continue
            for entry in folder.iterdir():
                if entry.is_symlink() and entry.exists():
                    for row in by_dir.get(entry.resolve(), []):
                        row['linked'].add(tool)

    def skill_inventory(self, bridge):
        reply = bridge.handle({'action': 'list', 'sources': [str(self.skills_home())]})
        ctx = bridge.base
        rows = reply['skills']
        self.skill_links(ctx, rows)
        tools = self.skill_modules['common'].TOOLS
        # Harnesses as the engine's discovery reports them, in its own order.
        found = {tool for row in rows for tool in row.get('tools', [])}
        harnesses = [tool for tool in tools if tool in found]
        home = self.skills_home().resolve()
        managed = {entry['name']: entry for entry in reply.get('library', [])}
        self.skill_paths = {row['id']: row['path'] for row in rows}
        skills = []
        for row in rows:
            locations = row.get('locations', [])
            in_home = Path(row['path']).resolve().is_relative_to(home)
            collections = list(dict.fromkeys(self.skill_collection(ctx, loc) for loc in locations))
            linked = [tool for tool in tools if tool in row['linked']]
            entry = managed.get(row['name'])
            skills.append(dict(
                id=row['id'], name=row['name'], description=' '.join(str(row.get('description') or '').split()),
                linked=linked, linkedNames=[self.skill_label(t) for t in linked],
                found=[self.skill_label(t) for t in row.get('tools', []) if t not in row['linked']],
                managed=list((entry or {}).get('linked_targets') or []),
                issues=[self.skill_label(t) for t in (row.get('link_issues') or {})],
                integrity=row.get('link_integrity') or '',
                errors=[self.skill_scrub(e) for e in row.get('errors', [])],
                warnings=[self.skill_scrub(w) for w in row.get('warnings', [])],
                library=in_home, source='Skills home' if in_home else (collections[0] if collections else 'Local collection'),
                collections=collections))
        counts = {tool: sum(1 for row in rows if tool in row['linked']) for tool in harnesses}
        sources = {}
        for skill in skills:
            for name in skill['collections']:
                sources[name] = sources.get(name, 0) + 1
        result = dict(skills=skills, harnesses=[dict(key=t, name=self.skill_label(t), linked=counts[t]) for t in harnesses],
                      sources=[dict(name=k, count=v) for k, v in sorted(sources.items(), key=lambda kv: (kv[0] != 'Skills home', kv[0]))],
                      restrictions=[dict(id=op['id'], tool=self.skill_label(op['tool']), names=op['names'])
                                    for op in reply.get('policy_operations', [])],
                      notes=[self.skill_scrub(w) for w in reply.get('warnings', [])
                             if not w.startswith('Inventory shows local discovery')],
                      scope='User-wide · all projects')
        self.skill_last = result
        return result

    def skill_change(self, ctx, change):
        # One change of the engine's plan in names: skill, tool, verb.
        path, tools = Path(change['path']), self.skill_modules['common'].TOOLS
        for tool in tools:
            target = ctx.target(tool)
            if path.parent in (target, target.resolve()):
                verb = {'link': 'Link', 'unlink': 'Unlink', 'remove': 'Remove'}.get(change['action'], change['action'].capitalize())
                return dict(kind=change['action'], skill=path.name, tool=self.skill_label(tool),
                            text=f'{verb} {path.name} {"to" if verb == "Link" else "from"} {self.skill_label(tool)}')
        if path.name == 'state.json':
            return dict(kind='record', skill='', tool='', text="Record the links in the skill manager's catalog")
        if change['action'] == 'create':
            return dict(kind='create', skill=path.name, tool='', text=f'Create the skill {path.name} in the skills home')
        return dict(kind=change['action'], skill=path.name, tool='', text=self.skill_scrub(change['reason']))

    def skill_preview(self, bridge, reply):
        preview = reply['preview']
        changes = [self.skill_change(bridge.base, change) for change in preview['changes']]
        compat = [self.skill_scrub(w) for w in preview.get('compatibilityWarnings', [])]
        notes = [self.skill_scrub(w) for w in preview.get('warnings', []) if w not in preview.get('compatibilityWarnings', [])]
        return dict(preview=dict(title=preview['title'], changes=changes, compatibility=compat, notes=notes,
                                 links=sum(1 for c in changes if c['kind'] in ('link', 'unlink'))),
                    token=reply['token'])

    def skills(self, request):
        bridge = self.skill_engine()
        action = request.get('action', 'list')
        tools = self.skill_modules['common'].TOOLS
        if action in ('list', 'rescan'):
            return self.skill_inventory(bridge)
        if action == 'cancel':
            bridge.handle({'action': 'cancel'})
            return {'cancelled': True}
        if action == 'validate':
            path = self.skill_paths.get(request.get('id', ''))
            if not path:
                raise ValueError('Choose a skill to check')
            info = bridge.handle({'action': 'validate', 'path': path})['validation']
            return {'validation': dict(name=info['name'], errors=[self.skill_scrub(e) for e in info['errors']],
                                       warnings=[self.skill_scrub(w) for w in info['warnings']])}
        if action == 'preview':
            if os.environ.get('SOLITON_AUTOMATION') == '1' and not os.environ.get('SOLITON_HARNESS_HOME'):
                raise ValueError('Test runs change skills only in a private harness folder')
            operation = request.get('operation')
            chosen = [t for t in request.get('tools', []) if t in tools]
            if operation == 'share':
                if request.get('path'):
                    paths = [os.path.expanduser(str(request['path']).strip())]
                else:
                    paths = [self.skill_paths[i] for i in request.get('ids', []) if i in self.skill_paths]
                if not paths:
                    raise ValueError('Choose one or more skills')
                if not chosen:
                    raise ValueError('Choose one or more tools')
                reply = bridge.handle({'action': 'preview', 'operation': 'share', 'paths': paths, 'tools': chosen})
            elif operation in ('remove', 'update'):
                reply = bridge.handle({'action': 'preview', 'operation': operation, 'names': request.get('names', [])})
            elif operation == 'create':
                name = str(request.get('name', '')).strip()
                reply = bridge.handle({'action': 'preview', 'operation': 'create', 'name': name,
                                       'path': str(self.skills_home() / name), 'description': request.get('description', '')})
            elif operation == 'disable':
                reply = bridge.handle({'action': 'preview', 'operation': 'disable', 'tool': request.get('tool', ''),
                                       'names': [str(request.get('name', '')).strip()]})
            elif operation == 'enable':
                reply = bridge.handle({'action': 'preview', 'operation': 'enable', 'operationId': request.get('operationId', '')})
            elif operation == 'recover':
                reply = bridge.handle({'action': 'preview', 'operation': 'recover'})
            else:
                raise ValueError('Unknown skills operation')
            return self.skill_preview(bridge, reply)
        if action == 'apply':
            if os.environ.get('SOLITON_AUTOMATION') == '1' and not os.environ.get('SOLITON_HARNESS_HOME'):
                raise ValueError('Test runs change skills only in a private harness folder')
            reply = bridge.handle({'action': 'apply', 'token': request.get('token', '')})
            applied = reply['applied']
            changes = [self.skill_change(bridge.base, change) for change in applied.get('changes', [])]
            result = self.skill_inventory(bridge)
            result['applied'] = dict(status=applied['status'], changes=changes)
            return result
        raise ValueError('Unknown skills operation')

    def handle(self, request):
        action = request['action']
        if action == 'plugins': return self.plugins(request.get('request', {}))
        if action == 'skills': return self.skills(request.get('request', {}))
        raise ValueError('Unknown settings operation')


def main():
    owner = Settings()
    while True:
        raw = sys.stdin.buffer.readline(1024 * 1024 + 1)
        if not raw:
            break
        request_id = None
        try:
            if len(raw) > 1024 * 1024:
                # Discard the remainder without accumulating an unbounded line.
                while raw and not raw.endswith(b'\n'):
                    raw = sys.stdin.buffer.readline(1024 * 1024 + 1)
                raise ValueError('Request is too large')
            request = json.loads(raw)
            request_id = request['id']
            result = {'id': request_id, 'result': owner.handle(request)}
        except Exception as error:
            result = {'id': request_id, 'error': str(error)[:4000]}
        print(json.dumps(result), flush=True)


if __name__ == '__main__':
    main()
