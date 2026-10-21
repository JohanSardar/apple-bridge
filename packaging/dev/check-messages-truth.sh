#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
directory=$(mktemp -d /tmp/ablab/w2-truth-fixtures.XXXXXX)
trap 'rm -rf "$directory"' EXIT
packaging/dev/build-truth.sh "$directory/AppleTruth.app" >/dev/null
python3 - "$directory" <<'PY'
import json,sqlite3,sys
from pathlib import Path
root=Path(sys.argv[1]); fixtures=Path('lab/fixtures/messages'); db=sqlite3.connect(root/'chat.db')
flags='is_audio_message date_played is_expirable schedule_type was_delivered_quietly did_notify_recipient is_spam has_unseen_mention is_stewie sent_or_received_off_grid is_sos'.split()
db.executescript('CREATE TABLE message(balloon_bundle_id TEXT,payload_data BLOB,attributedBody BLOB,subject TEXT,associated_message_type INTEGER DEFAULT 0,associated_message_guid TEXT,'+','.join(k+' INTEGER DEFAULT 0' for k in flags)+'); CREATE TABLE attachment(is_sticker INTEGER,hide_attachment INTEGER,emoji_image_content_identifier TEXT,sticker_user_info BLOB,mime_type TEXT,uti TEXT); CREATE TABLE chat(is_filtered INTEGER,properties BLOB); CREATE TABLE message_attachment_join(message_id INTEGER,attachment_id INTEGER); CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER); CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER,delete_date INTEGER);')
db.execute('INSERT INTO message(balloon_bundle_id,payload_data,attributedBody,subject,'+','.join(flags)+') VALUES('+','.join('?' for _ in range(4+len(flags)))+')', ['com.apple.messages.URLBalloonProvider',(fixtures/'rich-link.plist').read_bytes(),(fixtures/'rich-formatting.typedstream').read_bytes(),'Synthetic subject']+[1]*len(flags))
db.execute('INSERT INTO message(attributedBody,associated_message_guid) VALUES(?,?)',[(fixtures/'rich-genmoji.typedstream').read_bytes(),'p:2/synthetic-parent'])
db.execute('INSERT INTO message(balloon_bundle_id,payload_data) VALUES(?,?)',['com.apple.messages.MSMessageExtensionBalloonPlugin:synthetic',(fixtures/'rich-app.plist').read_bytes()])
db.execute('INSERT INTO attachment VALUES(1,1,?,?,?,?)',['synthetic-glyph',(fixtures/'rich-sticker.plist').read_bytes(),'image/png','public.png'])
db.execute('INSERT INTO attachment VALUES(0,0,NULL,NULL,?,?)',['image/png','public.png'])
db.executescript('INSERT INTO message_attachment_join VALUES(1,1),(1,2),(2,1),(3,2); INSERT INTO chat_recoverable_message_join VALUES(1,1,1);')
db.execute('INSERT INTO chat VALUES(2,?)',[(fixtures/'rich-chat.plist').read_bytes()])
db.executescript('INSERT INTO message DEFAULT VALUES; INSERT INTO message DEFAULT VALUES; INSERT INTO chat DEFAULT VALUES; INSERT INTO chat_message_join VALUES(2,4),(2,5); INSERT INTO chat_recoverable_message_join VALUES(1,4,1);')
db.commit(); db.close()
(root/'request.json').write_text(json.dumps({'source':'messages','rich_counts':True,'snapshot_path':str(root/'chat.db')}))
PY
open -n -W -g -a "$directory/AppleTruth.app" --args observe --request-file "$directory/request.json" --report "$directory/report.json"
python3 - "$directory/report.json" <<'PY'
import json,sys
report=json.load(open(sys.argv[1])); assert report['status']=='verified',report.get('reason')
counts=report['counts']; expected={k:1 for k in 'balloon.link balloon.app balloon.link_card balloon.app_caption balloon.link.title balloon.link.summary balloon.link.site_name balloon.link.video_url balloon.link.icon_attachment_id balloon.app.subcaption balloon.app.trailing_caption balloon.app.url audio audio.played audio.expirable schedule subject delivery.quietly delivery.notified_anyway is_spam has_unseen_mention via_satellite is_sos formatting genmoji attachment.is_sticker attachment.hidden attachment.genmoji attachment.sticker chat.unknown chat.is_spam chat.group_photo chat.read_receipts message.deleted_at chat.deleted_at'.split()}; expected.update({'balloon.payload_decoded':2,'balloon.image_reference':2})
assert all(counts.get(k)==v for k,v in expected.items()),{k:[counts.get(k),v] for k,v in expected.items() if counts.get(k)!=v}
print('all independent Messages Truth fixture counts passed')
PY
