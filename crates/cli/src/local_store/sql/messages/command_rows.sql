SELECT message_json FROM messages WHERE body_kind = 'tool' AND message_json LIKE '%"run":{%'
