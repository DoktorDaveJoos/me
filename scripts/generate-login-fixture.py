#!/usr/bin/env python3
"""Generate a deterministic 1PUX v3 export containing only synthetic test data."""
import json
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[1]
DESTINATION = ROOT / 'crates/me-core/tests/fixtures/synthetic-logins.1pux'


def login(index, title, archived=False):
    return {
        'uuid': f'synthetic-login-{index}', 'favIndex': 1 if index in (1, 3) else 0,
        'state': 'archived' if archived else 'active', 'categoryUuid': '001',
        'createdAt': 1750000000, 'updatedAt': 1760000000,
        'overview': {'title': title, 'url': f'https://service{index}.example/',
                     'urls': [{'label': 'Sign in', 'url': f'https://service{index}.example/'},
                              {'label': 'Account', 'url': f'https://service{index}.example/account'}],
                     'tags': ['Synthetic', 'Personal' if index < 5 else 'Work']},
        'details': {'loginFields': [
            {'name': 'email', 'designation': 'username', 'fieldType': 'E', 'value': f'alex{index}@example.com'},
            {'name': 'password', 'designation': 'password', 'fieldType': 'P', 'value': f'  SAMPLE-only-{index}-café-🔑  '}],
            'notesPlain': 'Synthetic account for ME. testing.\nThis is not a real password or account.',
            'sections': [{'title': 'Security', 'name': 'security', 'fields': [
                {'id': 'pin', 'title': 'Support PIN', 'value': {'concealed': '001234'}},
                {'id': 'otp', 'title': 'One-time password', 'value': {'totp': 'otpauth://totp/Synthetic?secret=JBSWY3DPEHPK3PXP&issuer=Synthetic'}},
                {'id': 'label', 'title': 'Account label', 'value': {'string': 'Test workspace'}},
                {'id': 'number', 'title': 'Seat count', 'value': {'number': 3}},
                {'id': 'address', 'title': 'Address', 'value': {'address': {'city': 'Berlin', 'zip': '00123'}}}]}],
            'passwordHistory': [{'value': 'SAMPLE-previous-password', 'time': 1750000000}],
            'futureField': {'preserve': 'Synthetic unknown metadata'}}}


items = [login(i, title, i == 7) for i, title in enumerate([
    'Atlas Mail', 'Aster Cloud', 'Codeforge', 'Design workspace', 'Greenhouse',
    'Northstar Banking · business account with a deliberately long title',
    'Studio München — 日本語', 'Travel account (old)', 'Website without saved credentials'
], 1)]
items[-1]['details']['loginFields'] = []
items[-1]['overview']['urls'] = []
items[-1]['overview']['url'] = ''
items[0]['details']['documentAttributes'] = {'documentId': 'synthetic-attachment', 'fileName': 'Recovery instructions.txt'}
card = login(20, 'Synthetic card — preserved outside Logins')
card['categoryUuid'] = '002'
note = login(21, 'Synthetic secure note — preserved outside Logins')
note['categoryUuid'] = '003'
# Identical item UUID in a different vault is a distinct login.
shared = login(1, 'Atlas Mail — shared')
account = {'attrs': {'uuid': 'synthetic-account', 'accountName': 'ME. Demo', 'email': 'alex@example.com'},
           'vaults': [
               {'attrs': {'uuid': 'synthetic-personal', 'name': 'Personal', 'type': 'P'}, 'items': items[:4] + [card, note]},
               {'attrs': {'uuid': 'synthetic-work', 'name': 'Work', 'type': 'U'}, 'items': items[4:]},
               {'attrs': {'uuid': 'synthetic-shared', 'name': 'Shared', 'type': 'E'}, 'items': [shared]}]}
DESTINATION.parent.mkdir(parents=True, exist_ok=True)
with ZipFile(DESTINATION, 'w') as archive:
    for name, content in [
        ('export.attributes', json.dumps({'version': 3, 'description': '1Password Unencrypted Export', 'createdAt': 1760000000})),
        ('export.data', json.dumps({'accounts': [account]}, ensure_ascii=False)),
        ('files/synthetic-attachment__Recovery instructions.txt', 'SYNTHETIC attachment. No real secrets.\n')
    ]:
        entry = ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
        entry.compress_type = ZIP_DEFLATED
        archive.writestr(entry, content.encode('utf-8'))
print(f'Created {DESTINATION}: 10 logins, 2 other entries, 3 vaults, 1 attachment.')
