#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$1
rm -rf "$root"
mkdir -p "$root/Attachments"
sqlite3 "$root/Calendar.sqlitedb" < "$here/schema.sql"
printf 'pdf-bytes-1' > "$root/Attachments/a.pdf"
printf 'hello' > "$root/Attachments/b.txt"
