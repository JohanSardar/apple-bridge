CREATE TABLE Calendar (ROWID INTEGER PRIMARY KEY, UUID TEXT);
CREATE TABLE Location (ROWID INTEGER PRIMARY KEY, title TEXT, address TEXT, latitude REAL, longitude REAL, radius REAL);
CREATE TABLE Participant (ROWID INTEGER PRIMARY KEY, status INTEGER);
CREATE TABLE Conference (owner_id INTEGER, url TEXT);
CREATE TABLE CalendarItem (ROWID INTEGER PRIMARY KEY, unique_identifier TEXT, UUID TEXT, external_id TEXT, calendar_id INTEGER, location_id INTEGER, self_attendee_id INTEGER, travel_time INTEGER, travel_advisory_behavior INTEGER, invitation_status INTEGER, privacy_level INTEGER, priority INTEGER, conference_url TEXT);
CREATE TABLE AttachmentFile (ROWID INTEGER PRIMARY KEY, UUID TEXT, filename TEXT, local_path TEXT, file_size INTEGER, format TEXT);
CREATE TABLE Attachment (owner_id INTEGER, file_id INTEGER);
INSERT INTO Calendar VALUES (1, 'AAAAAAAA-0000-0000-0000-000000000001'), (2, 'AAAAAAAA-0000-0000-0000-000000000002');
INSERT INTO Location VALUES (1, 'HQ', '1 Main St', 37.5, -122.5, 100.0);
INSERT INTO Participant VALUES (1, 2), (2, 3);
INSERT INTO Conference VALUES (2, 'https://meet.example/solo'), (7, 'https://meet.example/one'), (7, 'https://meet.example/two');
INSERT INTO CalendarItem VALUES
 (1, 'ID-A', 'UUID-A', 'EXT-A', 1, 1, 1, 0, 1, 2, 0, 5, 'https://meet.example/a'),
 (2, 'ID-B', 'UUID-B', 'EXT-B', 1, NULL, 2, 900, 0, 0, 1, 0, NULL),
 (3, 'ID-C', 'UUID-C', 'EXT-DUP', 1, NULL, NULL, NULL, 0, 0, 0, 0, NULL),
 (4, 'ID-D', 'UUID-D', 'EXT-DUP', 1, NULL, NULL, NULL, 0, 0, 0, 0, NULL),
 (5, 'ID-E', 'UUID-E', 'EXT-E', 1, NULL, NULL, 0, 0, 0, 0, 0, 'ftp://files.example/x'),
 (6, 'ID-F', 'UUID-F', 'EXT-F', 2, NULL, NULL, 0, 0, 0, 0, 0, ''),
 (7, 'ID-G', 'UUID-G', 'EXT-G', 1, NULL, NULL, 0, 0, 0, 0, 0, NULL),
 (8, 'ID-H', 'UUID-H', 'EXT-H', 1, NULL, NULL, 0, 0, 0, 0, 0, NULL);
INSERT INTO AttachmentFile VALUES
 (1, 'F1', 'a.pdf', 'Attachments/a.pdf', 11, 'application/pdf'),
 (2, 'F2', 'b.txt', 'Attachments/b.txt', 5, 'text/plain'),
 (3, 'F3', 'gone.png', 'Attachments/gone.png', 9, 'image/png'),
 (4, 'F4', 'hosts', '/etc/hosts', 9, 'text/plain');
INSERT INTO Attachment VALUES (8, 1), (8, 2), (1, 2), (3, 3), (4, 4);
