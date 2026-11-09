-- list_threads / get_thread_detail / actions : lecture des messages d'une conversation par date,
-- et appartenance conversation → dossier. (`idx_messages_thread` devient un préfixe du premier.)
DROP INDEX idx_messages_thread;
CREATE INDEX idx_messages_thread_date ON messages(thread_id, date, id);
CREATE INDEX idx_messages_folder_thread ON messages(folder_id, thread_id);
