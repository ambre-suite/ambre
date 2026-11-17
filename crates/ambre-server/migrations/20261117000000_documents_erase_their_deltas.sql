-- The database erases the delta rows of a deleted document.
--
-- `updates` carries no foreign key to `documents`, so that an insert locks no
-- document row (`SRV-274`). Without the key, nothing in the schema removed the
-- deltas of a deleted document: each delete path of the relay had to. A path
-- that forgot it left orphan rows. This trigger removes them on every path,
-- plain SQL and a cascade included (`SRV-275`).
--
-- The trigger fires on a delete of `documents` alone, so an insert into
-- `updates` pays nothing. It is per row, and its query reads the primary key
-- of `updates` by its first column, the document. A per-statement trigger over
-- a transition table carries no statistics, and its plan may scan the table.
--
-- A write past its checks when the tombstone was set may still insert a row
-- after the document went. No trigger on `documents` sees that row, so the
-- second sweep of the delete still removes it (`SRV-273`).
CREATE FUNCTION erase_document_deltas() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM updates WHERE document_id = OLD.id;
    RETURN NULL;
END;
$$;

CREATE TRIGGER documents_erase_their_deltas
    AFTER DELETE ON documents
    FOR EACH ROW
    EXECUTE FUNCTION erase_document_deltas();
