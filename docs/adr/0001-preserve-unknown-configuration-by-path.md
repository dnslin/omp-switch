# Preserve unknown configuration by path

OMP Switch edits the parsed YAML tree in place and may mutate only user-selected paths under `models.yml.providers` and `config.yml.modelRoles`. Every untouched path must retain the same value after serialization and reparse; otherwise the write is rejected. This keeps unsupported OMP configuration safe without requiring comment- or formatting-preserving YAML edits, which the MVP explicitly does not guarantee.
