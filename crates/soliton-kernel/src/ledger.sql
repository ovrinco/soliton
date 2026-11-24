-- Soliton data plane, schema version 1 (frozen design: T001 DATA_PLANE.md and example.sql).
-- Applied once by Ledger::migrate to a new ledger.sqlite; later changes are additive, keyed on user_version.
CREATE TABLE kind (
  kind          TEXT PRIMARY KEY,
  writer        TEXT NOT NULL,
  default_class TEXT NOT NULL CHECK (default_class IN ('public','internal','personal_sensitive','restricted')),
  unit          TEXT,
  description   TEXT NOT NULL
);

CREATE TABLE observation (
  id            TEXT PRIMARY KEY CHECK (id GLOB 'obs_*'),
  kind          TEXT NOT NULL REFERENCES kind(kind),
  observed_on   TEXT NOT NULL CHECK (observed_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
  observed_at   TEXT,
  item_id       TEXT,
  category      TEXT,
  subject       TEXT NOT NULL,
  quantity      REAL,
  unit          TEXT,
  amount_minor  INTEGER,
  currency      TEXT CHECK (currency IS NULL OR length(currency) = 3),
  data_class    TEXT NOT NULL CHECK (data_class IN ('public','internal','personal_sensitive','restricted')),
  writer        TEXT NOT NULL,
  source_ref    TEXT,
  recorded_at   TEXT NOT NULL,
  superseded_by TEXT REFERENCES observation(id),
  private       INTEGER CHECK (private IS NULL OR private IN (0,1)),
  extra         TEXT CHECK (extra IS NULL OR json_valid(extra))
);
CREATE INDEX observation_day  ON observation(observed_on, kind);
CREATE INDEX observation_item ON observation(item_id);
CREATE INDEX observation_cat  ON observation(category);

CREATE VIEW current_observation AS
  SELECT o.*, date(o.observed_on, '-6 days', 'weekday 1') AS week
  FROM observation o WHERE o.superseded_by IS NULL;

CREATE TABLE private_mark (
  scope TEXT NOT NULL CHECK (scope IN ('item','category','goal','module')),
  key   TEXT NOT NULL,
  PRIMARY KEY (scope, key)
);

CREATE VIEW visible_observation AS
  SELECT o.* FROM current_observation o
  WHERE coalesce(o.private,0) = 0
    AND NOT EXISTS (SELECT 1 FROM private_mark m WHERE
          (m.scope='item' AND m.key=o.item_id) OR (m.scope='category' AND m.key=o.category)
       OR (m.scope='goal' AND m.key=json_extract(o.extra,'$.goal_id')) OR (m.scope='module' AND m.key=o.writer));

CREATE VIEW shareable_observation AS
  SELECT * FROM current_observation WHERE data_class IN ('public','internal');

INSERT INTO kind VALUES
 ('check_in',    'habits',  'internal',           'done',  'A habit, bundle step or planner item kept (1) or missed (0) on a day'),
 ('meal',        'kitchen', 'personal_sensitive', 'g',     'Something eaten or cooked, with where it came from'),
 ('purchase_line','finance','personal_sensitive', 'item',  'One line of a receipt or statement'),
 ('measurement', 'health',  'personal_sensitive', NULL,    'A chosen health measure with its unit');

PRAGMA user_version = 1;
