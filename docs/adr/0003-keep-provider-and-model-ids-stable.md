# Keep Provider and Model IDs stable

Existing Provider IDs and Model IDs are immutable in the MVP. OMP resolves identifiers without regard to letter case and configuration outside `modelRoles` can also contain model selectors, so a rename cannot be made safe while OMP Switch intentionally limits writes to `providers` and `modelRoles`. Users change an ID by creating a new object, updating references explicitly, and deleting the old object after reference checks pass.
