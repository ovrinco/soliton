-- Soliton data plane, schema version 2 (S4a design T007 S4_DESIGN.md, built in S4b T008).
-- Additive on version 1: five kind rows, two module marks and one small table holding the
-- statement column mapping of each bank. No table, column or view of version 1 changes.
-- Applied in one transaction by Ledger::migrate (new ledger) or Ledger::upgrade (version 1).
--
-- Undo to version 1, in one transaction, only while no row of the five kinds exists
-- (the kind foreign key refuses otherwise; supersede or remove those rows first):
--   DELETE FROM kind WHERE kind IN ('inventory','equipment','cook_profile','batch_plan','measure_definition');
--   DELETE FROM private_mark WHERE scope='module' AND key IN ('finance','health');  -- unless set by hand before
--   DROP TABLE statement_mapping;
--   PRAGMA user_version = 1;
-- Purchase lines imported from statements are kind purchase_line (version 1) and stay.
INSERT INTO kind VALUES
 ('inventory',  'kitchen','internal',          'item','One lot in the kitchen now: quantity, unit, where, and when it goes off (extra.expires_on)'),
 ('equipment',  'kitchen','internal',          'item','One piece of kitchen equipment with its capacity'),
 ('cook_profile','kitchen','personal_sensitive',NULL, 'One standing constraint of the cook: diet, portions, session length, skill'),
 ('batch_plan', 'kitchen','internal',          'portion','One dish of a large mise en place plan, with the lots it uses'),
 ('measure_definition','health','personal_sensitive',NULL,'A health measure the operator chose to log, with its unit (proposed, R-24 H1)');

-- Finance and health private from the first capture.
INSERT OR IGNORE INTO private_mark VALUES ('module','finance'), ('module','health');

-- One row per bank: which statement columns mean what. Set once, read at each import.
CREATE TABLE statement_mapping (
  bank        TEXT PRIMARY KEY CHECK (length(trim(bank)) > 0),
  mapping     TEXT NOT NULL CHECK (json_valid(mapping)),
  recorded_at TEXT NOT NULL
);

PRAGMA user_version = 2;
