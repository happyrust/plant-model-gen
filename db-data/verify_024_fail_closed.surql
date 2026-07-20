-- Spec 024 / SC-002 fail-closed fixture.
--
-- Run this file with `surreal sql` against an isolated versioned instance.
-- The final statement MUST return a statement-level error. The application
-- path must convert that Response::check() error to a non-zero process exit.
-- A runner must then verify that:
--   1. `version_commit_state:[24024, 2]` remains `pending`;
--   2. no `sesno_version_anchor` row exists for dbnum 24024 / sesno 2.
-- This mirrors commit_version: preparing -> mutation failure -> pending, with no
-- anchor publication. The deliberate statement-level error also proves callers
-- must invoke Response::check(), not merely await the transport.

USE NS test DB verify_024_fail_closed;

DEFINE TABLE IF NOT EXISTS sesno_version_anchor SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD IF NOT EXISTS sesno ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD IF NOT EXISTS source ON TABLE sesno_version_anchor TYPE string
    ASSERT $value IN ['full', 'incremental', 'model_gen'];
DEFINE FIELD IF NOT EXISTS anchored_at ON TABLE sesno_version_anchor TYPE datetime
    DEFAULT time::now();

DEFINE TABLE IF NOT EXISTS version_commit_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS to_sesno ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS status ON TABLE version_commit_state TYPE string
    ASSERT $value IN ['preparing', 'pending', 'committed'];
DEFINE FIELD IF NOT EXISTS last_error ON TABLE version_commit_state TYPE option<string>;

DEFINE TABLE IF NOT EXISTS verify_024_illegal SCHEMAFULL;

DELETE sesno_version_anchor WHERE dbnum = 24024;
DELETE version_commit_state WHERE dbnum = 24024;
DELETE verify_024_illegal;

UPSERT version_commit_state:[24024, 2] SET
    dbnum = 24024,
    to_sesno = 2,
    status = 'preparing',
    last_error = NONE;

UPDATE version_commit_state:[24024, 2] SET
    status = 'pending',
    last_error = 'injected statement-level mutation failure';

-- Intentional SCHEMAFULL violation. Expected: Response::check() returns Err.
CREATE verify_024_illegal:1 SET undeclared_field = 'must fail';
