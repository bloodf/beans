SELECT CAST(MAX(created_at) AS INTEGER) FROM messages WHERE author_kind = 'you'
