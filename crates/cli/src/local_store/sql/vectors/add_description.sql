-- ?1 message, ?2 attachment, ?3 model, ?4 text. The first description of an image stays.
INSERT OR IGNORE INTO image_descriptions(message_id, attachment_id, model, text) VALUES (?1, ?2, ?3, ?4)
