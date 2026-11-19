-- Derived search tables of an older `SEARCH_INDEX_VERSION`, dropped before `search.sql`
-- creates the current ones and reseeds them. Their triggers go with them; image descriptions,
-- which cost tokens to make, are not derived and stay.
DROP TABLE IF EXISTS message_vectors;
DROP TABLE IF EXISTS message_vector_deletions;
DROP TABLE IF EXISTS message_vector_spaces;
DROP TABLE IF EXISTS message_vector_space;
DROP TABLE IF EXISTS message_search;
