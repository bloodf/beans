-- ?1 table, ?2 column: whether an older store already has the column.
SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)
