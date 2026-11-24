-- The settings of a user live in one document whose only member is that user
-- (`SETTINGS-001`). The relay keeps one pointer per account, set once, so two
-- first unlocks at once receive one document (`SETTINGS-002`, `SETTINGS-009`).
-- The pointer is all the relay learns of the settings: the document is sealed
-- like any other (`ZK-053`, `ZK-054`).
ALTER TABLE documents DROP CONSTRAINT documents_doc_type_check;
ALTER TABLE documents ADD CONSTRAINT documents_doc_type_check
    CHECK (doc_type IN ('prose', 'trame', 'toile', 'murmure', 'settings'));

ALTER TABLE users ADD COLUMN settings_document UUID
    REFERENCES documents (id) ON DELETE SET NULL;

-- One document holds the settings of one account at most.
CREATE UNIQUE INDEX users_settings_document_unique ON users (settings_document);
