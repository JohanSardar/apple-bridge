import datetime
import os
import pathlib
import plistlib
import sqlite3

root = pathlib.Path(__file__).parent
connection = sqlite3.connect('file:' + os.path.expanduser('~/Library/Messages/chat.db') + '?mode=ro', uri=True)
allowed = {'URL', 'caption', 'subcaption', 'secondary-subcaption', 'tertiary-subcaption', 'ldtext', 'an', 'ai', 'appid', 'image-title', 'image-subtitle', 'layoutClass', 'liveLayoutInfo', 'requiredCapabilities', 'sessionIdentifier', 'userInfo'}
for label, condition in [('link', "balloon_bundle_id='com.apple.messages.URLBalloonProvider'"), ('app', "balloon_bundle_id LIKE '%MSMessageExtensionBalloonPlugin%' AND payload_data IS NOT NULL")]:
    raw = connection.execute('SELECT payload_data FROM message WHERE ' + condition + ' ORDER BY ROWID LIMIT 1').fetchone()[0]
    value = plistlib.loads(raw)
    objects = value['$objects']
    key_indices = set()
    class_indices = set()
    for item in objects:
        if isinstance(item, dict):
            for key in item.get('NS.keys', []):
                if isinstance(key, plistlib.UID) and objects[key.data] in allowed:
                    key_indices.add(key.data)
            if '$classname' in item:
                class_indices.update(i for i, other in enumerate(objects) if isinstance(other, str) and other in item.get('$classes', []) + [item['$classname']])
    def scrub(item, key=''):
        if isinstance(item, dict):
            return {k: scrub(v, k) for k, v in item.items()}
        if isinstance(item, list):
            return [scrub(v, key) for v in item]
        if isinstance(item, str):
            if item == '$null' or key in ('$classname', '$classes', '$archiver'):
                return item
            return 'https://example.invalid/preview' if item.startswith(('http:', 'https:')) else 'Synthetic preview'
        if isinstance(item, bytes):
            return bytes(len(item))
        if isinstance(item, datetime.datetime):
            return datetime.datetime(2001, 1, 1)
        if isinstance(item, bool):
            return False
        if isinstance(item, (int, float)) and key not in ('$version', 'version', 'richLinkImageAttachmentSubstituteIndex'):
            return 0
        return item
    scrubbed = scrub(value)
    for i in key_indices | class_indices:
        scrubbed['$objects'][i] = objects[i]
    (root / ('rich-air-' + label + '.plist')).write_bytes(plistlib.dumps(scrubbed, fmt=plistlib.FMT_BINARY))
print('scrubbed rich archive fixtures captured')
