-- 0005: a stable data key (P17). Session logs and command history are encrypted with it instead of the
-- password-derived key, so changing the master password no longer makes old logs unreadable.
-- `wrapped_data_key` = nonce || ciphertext of the data key, sealed with the password-derived key.
-- NULL on vaults made before this migration: the first unlock adopts the current key as the data key,
-- so existing logs stay readable.
ALTER TABLE vaults ADD COLUMN wrapped_data_key BLOB;
-- Command history moves to an encrypted column. Old plain rows are encrypted on the next use.
ALTER TABLE command_history ADD COLUMN command_enc BLOB;
