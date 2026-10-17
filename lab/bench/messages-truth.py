import hashlib
import json
import os
import sqlite3
import sys

with open(sys.argv[1]) as source:
    observed = json.load(source)
connection = sqlite3.connect("file:" + os.path.expanduser("~/Library/Messages/chat.db") + "?mode=ro", uri=True)
expected = {}
query = """SELECT COALESCE(c.guid,''),COUNT(DISTINCT m.guid)
FROM message m LEFT JOIN chat_message_join j ON j.message_id=m.ROWID LEFT JOIN chat c ON c.ROWID=j.chat_id
WHERE COALESCE(m.associated_message_type,0) NOT BETWEEN 2000 AND 2007
AND COALESCE(m.associated_message_type,0) NOT BETWEEN 3000 AND 3007
GROUP BY COALESCE(c.guid,'')"""
for chat, count in connection.execute(query):
    expected[hashlib.sha256(chat.encode()).hexdigest()] = count
passed = observed == expected
print(json.dumps({"native_chat_counts_equal_readonly_sqlite": passed}))
sys.exit(0 if passed else 1)
