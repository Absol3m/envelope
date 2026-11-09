-- find_thread() cherche les réponses arrivées avant leur parent via in_reply_to.
CREATE INDEX idx_messages_in_reply_to ON messages(account_id, in_reply_to);
