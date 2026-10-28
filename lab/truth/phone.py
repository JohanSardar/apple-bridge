import datetime
import json
import os
import sqlite3

path = os.path.expanduser('~/Library/Application Support/CallHistoryDB/CallHistory.storedata')
connection = sqlite3.connect('file:' + path + '?mode=ro', uri=True)
cutoff = datetime.datetime.now(datetime.timezone.utc).timestamp() - 30 * 86400
rows = {}
for key, address, name, date, duration, originated, answered, provider, kind in connection.execute('SELECT Z_PK,ZADDRESS,ZNAME,ZDATE,ZDURATION,ZORIGINATED,ZANSWERED,ZSERVICE_PROVIDER,ZCALLTYPE FROM ZCALLRECORD WHERE ZDATE >= ?', (cutoff - 978307200,)):
    service = 'FaceTime video' if kind == 8 else 'FaceTime audio'
    if 'facetime' not in (provider or '').lower():
        service = 'cellular'
    rows[str(key)] = dict(direction='outgoing' if originated else 'incoming' if answered else 'missed', handle=address or '', name=name or '', date=int((date + 978307200) * 1000), duration=duration or 0, service=service, answered=bool(answered))
print(json.dumps(rows))
