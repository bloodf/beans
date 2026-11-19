-- ?1 an attachment whose bytes just arrived on this Runner: the messages showing it are read
-- again by the image and describe queues. The version bump keeps a pass already under way from
-- taking them off with its older read.
UPDATE message_search SET image_pending = 1, describe_pending = 1, image_version = image_version + 1
WHERE images != '' AND instr(char(10) || images || char(10), char(10) || ?1 || char(10)) > 0
