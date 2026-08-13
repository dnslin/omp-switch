# Recover cross-file configuration transactions

Changes that affect both `models.yml` and `config.yml` use one recoverable transaction with shared backups, expected final hashes, and a durable manifest. On startup, matching final hashes complete transaction cleanup; any other state is copied aside and both files are restored from the same transaction backups. This prevents a crash between file replacements from leaving model definitions and role references partially updated.
