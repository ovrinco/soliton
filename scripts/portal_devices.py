#!/usr/bin/env python3
"""Read-only projection of the installed companion Store.devices schema.

No pairing, credentials, database creation, migration, or service startup.
Enrollment and last contact are evidence; neither implies a connected phone.
"""
import json
import os
from pathlib import Path
import sqlite3
import time


def snapshot(path=None):
    if path is None:
        explicit = os.environ.get('SOLITON_COMPANION_DB')
        if explicit:
            path = Path(explicit)
        else:
            config_path = Path.home() / '.config/soliton/connector.json'
            if not config_path.exists():
                return {'configured': False, 'items': []}
            config = json.loads(config_path.read_text())
            if config.get('kind') != 'soliton-connector-config/1':
                raise ValueError('Unsupported companion configuration; existing state was preserved.')
            path = Path(config['bridge_state']) / 'transport.sqlite3'
    path = Path(path).absolute()
    if not path.is_file():
        raise ValueError('The configured companion state is unavailable.')
    with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True, timeout=2) as db:
        db.row_factory = sqlite3.Row
        db.execute('PRAGMA query_only=ON')
        schema = db.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
        if schema is None or schema['value'] != '1':
            raise ValueError('Unsupported companion state version; existing state was preserved.')
        # Exact public field projection used by the installed Store.devices().
        rows = [dict(row) for row in db.execute(
            'SELECT id,label,created,expires,last_seen,revoked FROM sessions ORDER BY created DESC')]
    now = time.time()
    for row in rows:
        row['state'] = 'Revoked' if row['revoked'] else 'Expired' if row['expires'] <= now else 'Enrolled'
    return {'configured': True, 'items': rows, 'observed_at': now}


if __name__ == '__main__':
    try:
        print(json.dumps({'result': snapshot()}))
    except (OSError, ValueError, KeyError, sqlite3.Error):
        # Do not expose credentials, connection URLs, or raw configuration.
        print(json.dumps({'error': 'Companion device state could not be read. Existing pairing is unchanged.'}))
