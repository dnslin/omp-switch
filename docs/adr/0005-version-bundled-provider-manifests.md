# Version bundled Provider manifests

OMP Switch prevents Custom Provider IDs from colliding with OMP bundled Provider IDs by shipping a read-only manifest generated from the official `pi-catalog` for each explicitly covered OMP version. It does not use `omp models ls` because that command exposes only currently available authenticated models and can include user configuration, caches, and extensions. When the selected OMP version has no matching manifest, Provider and model management is read-only while unrelated configuration surfaces remain available.
