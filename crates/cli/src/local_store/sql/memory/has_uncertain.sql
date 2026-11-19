SELECT EXISTS(SELECT 1 FROM memory_deliveries WHERE state IN ('submitted','processing','delivery_unknown')
AND json_extract(json,'$.scope.namespace')=?1 AND json_extract(json,'$.scope.connection_id')=?2)
