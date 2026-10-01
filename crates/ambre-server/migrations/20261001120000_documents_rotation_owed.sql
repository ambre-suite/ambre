-- `SRV-122`: a removal posted without a rotation leaves its document owing one.
--
-- Every key the document held is one the departed member holds too. While the
-- debt stands, the relay refuses every frame (`SRV-123`), and any member with
-- write access may rotate (`SRV-144`). A rotation that lands clears it
-- (`SRV-124`).
ALTER TABLE documents ADD COLUMN rotation_owed BOOLEAN NOT NULL DEFAULT false;
