-- Coverage shares storage samples' retention and backup classification.
ALTER TABLE operation_storage_samples ADD COLUMN worktrees_covered INTEGER CHECK(worktrees_covered >= 0);
ALTER TABLE operation_storage_samples ADD COLUMN worktrees_expected INTEGER CHECK(worktrees_expected >= 0);
