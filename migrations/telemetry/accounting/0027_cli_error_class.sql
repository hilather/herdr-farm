-- Fixed diagnostic metadata only; retained with the existing CLI row.
ALTER TABLE cli_invocations ADD COLUMN error_class TEXT
    CHECK(error_class IS NULL OR error_class IN ('usage','precondition','not_found','store_busy','internal'));
