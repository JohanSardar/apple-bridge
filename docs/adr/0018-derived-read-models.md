# Views are served from derived read models, not from record queries

`records.query` pages by id only, so it cannot answer "chats by recent activity with unread counts" or "this chat's messages in time order". Derived tables in `state.db` are updated in the same transaction as each record apply and rebuilt on snapshot install. They serve `messages.chats`, `messages.thread` and `messages.collection` on every machine. They are never journaled and never authoritative, so dropping them loses nothing.

Chat pages traverse the activity index in display order and join each candidate to its chat record by primary key. The explicit join order keeps a bounded first page from scanning and sorting every chat. Participant-based Known Senders checks likewise start with that chat’s participants and look up each handle by primary key.
