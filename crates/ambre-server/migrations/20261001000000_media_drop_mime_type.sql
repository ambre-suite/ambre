-- `SYNC-053`: an upload tells the relay nothing about a blob beyond its size.
--
-- The media type travelled with the confirmation and was stored beside the
-- document. It is outside the set `ZK-007` allows, and the application already
-- keeps its own, so the column goes.
ALTER TABLE media DROP COLUMN mime_type;
