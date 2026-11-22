-- A CapsuleBin 1.0 database, as SQLite dumps it. Read by tests/upgrade.rs.
--
-- Written by the 1.0 binary itself, through its own commands and the HTTP API,
-- so `_sqlx_migrations` carries the checksum 1.0 records for 0001_init.sql.
-- It may be regenerated (with the script below) only before the 1.0 tag. After
-- the tag it stands for databases in the field: never regenerate or edit it.
--
--   EBAQEBAQEBAQEBAQEBAQEA  active; tokens describe-10, reveal-10, delete-10
--   EREREREREREREREREREREQ  burn after retrieval, already retrieved (tombstone)
--   EhISEhISEhISEhISEhISEg  password header; tokens describe-12, reveal-12, delete-12
--   ExMTExMTExMTExMTExMTEw  disabled by moderation (spam); tokens describe-13, reveal-13;
--                           one report about it, closed with action_taken
--   FBQUFBQUFBQUFBQUFBQUFA  expired after one second and removed by `prune`:
--                           no row, only its entry in retired_ids
--
-- The ciphertext of paste NN is 0x00 0xFF followed by "fixture-ciphertext-NN".
-- Every paste was created with the active database token. Creation tokens:
--
--   01M81WJGCMV3655M9FDWASE284  active, label "fixture active";
--                               token 4ZB7KtpKEH2_UScS3jlFAieRicESHEACLOPRl0t64yc
--   01M81WJGCTPRMWQD9FATJPRC4M  revoked, label "fixture revoked";
--                               token ux393DSUXF2Ulm9n065lWrO-Sdxx87632f45J5xGh0g
--
-- Written on 2026-11-22 (created_at 1795359850) by this script, run as
-- `generate.sh target/release/capsulebin dump.sql` (needs curl and python3;
-- Python's iterdump stands in for `sqlite3 <db> .dump`, same statements), with
-- this comment put in front of the dump and the tokens, ids and timestamps
-- above and in tests/upgrade.rs replaced by the new ones:
--
--   #!/usr/bin/env bash
--   # Writes the upgrade fixture: ./generate.sh <capsulebin binary> <output .sql>
--   set -euo pipefail
--   BIN=$(realpath "$1"); OUT=$(realpath -m "$2")
--   DIR=$(mktemp -d); DB="$DIR/capsulebin.db"; PORT=18089; URL="http://127.0.0.1:$PORT"
--   cat > "$DIR/capsulebin.toml" <<EOF
--   [server]
--   bind = "127.0.0.1:$PORT"
--   public_url = "https://paste.example"
--   [database]
--   path = "$DB"
--   [creation]
--   mode = "token"
--   EOF
--   cb() { "$BIN" --config "$DIR/capsulebin.toml" "$@"; }
--
--   # Verifier of a secret: base64url(SHA-256(domain || secret)).
--   verifier() { python3 -c 'import sys,hashlib,base64;print(base64.urlsafe_b64encode(hashlib.sha256(sys.argv[1].encode()+sys.argv[2].encode()).digest()).rstrip(b"=").decode())' "$1" "$2"; }
--   b64() { python3 -c 'import sys,base64;print(base64.urlsafe_b64encode(sys.stdin.buffer.read()).rstrip(b"=").decode())'; }
--   # Paste NN has the id of 16 bytes 0xNN, the tokens describe-NN, reveal-NN, delete-NN
--   # and the ciphertext 0x00 0xFF "fixture-ciphertext-NN".
--   pid() { python3 -c 'import sys,base64;print(base64.urlsafe_b64encode(bytes([int(sys.argv[1],16)]*16)).rstrip(b"=").decode())' "$1"; }
--   HEADER='{"envelope_format":1,"content_format":"capsule-file-v1","cipher":"AES-256-GCM","nonce":"AAECAwQFBgcICQoL","hkdf_salt":"EBESExQVFhcYGRobHB0eHw"}'
--   PASSWORD_HEADER='{"envelope_format":1,"content_format":"capsule-file-v1","cipher":"AES-256-GCM","nonce":"AAECAwQFBgcICQoL","hkdf_salt":"EBESExQVFhcYGRobHB0eHw","password":{"required":true,"kdf":"PBKDF2-HMAC-SHA-256","salt":"ICEiIyQlJicoKSorLC0uLw","iterations":600000}}'
--
--   ACTIVE_TOKEN=$(cb token create --label "fixture active" | sed -n 2p | tr -d ' ')
--   REVOKED=$(cb token create --label "fixture revoked")
--   REVOKED_ID=$(printf '%s\n' "$REVOKED" | sed -n 's/^id: //p')
--   printf 'active token: %s\nrevoked token: %s\n' "$ACTIVE_TOKEN" "$(printf '%s\n' "$REVOKED" | sed -n 2p | tr -d ' ')"
--
--   "$BIN" --config "$DIR/capsulebin.toml" serve &> "$DIR/serve.log" & SERVER=$!
--   trap 'kill $SERVER 2>/dev/null || true' EXIT
--   until curl -sf "$URL/health/live" >/dev/null; do sleep 0.2; done
--
--   create() { # create NN header extra-json
--     local nn=$1 header=$2 extra=$3
--     curl -sf -X POST "$URL/api/v1/pastes" -H "Authorization: Bearer $ACTIVE_TOKEN" \
--       -H 'Content-Type: application/json' --data-binary @- <<EOF
--   {"id":"$(pid "$nn")","format_version":1,"public_header":"$(printf '%s' "$header" | b64)",
--    "ciphertext":"$( (printf '\000\377'; printf 'fixture-ciphertext-%s' "$nn") | b64)",
--    "reveal_verifier":"$(verifier '' "reveal-$nn")","delete_verifier":"$(verifier capsule/v1/delete "delete-$nn")",
--    "describe_verifier":"$(verifier '' "describe-$nn")"$extra}
--   EOF
--     echo
--   }
--   create 10 "$HEADER" ''
--   create 11 "$HEADER" ',"burn_after_retrieval":true'
--   create 12 "$PASSWORD_HEADER" ''
--   create 13 "$HEADER" ''
--   create 14 "$HEADER" ',"expires_in":1'
--
--   # Burn 11 by retrieving it.
--   curl -sf -X POST "$URL/api/v1/pastes/$(pid 11)/retrieve" -H 'Content-Type: application/json' \
--     --data-binary "{\"describe_token\":\"$(printf describe-11 | b64)\",\"reveal_token\":\"$(printf reveal-11 | b64)\"}" >/dev/null
--   # Report 13, disable it, close the report.
--   curl -sf -X POST "$URL/api/v1/reports" -H 'Content-Type: application/json' \
--     --data-binary "{\"paste_id\":\"$(pid 13)\",\"reason\":\"spam\",\"description\":\"fixture report\",\"contact\":\"reporter@example.org\"}"
--   echo
--   cb moderation disable "$(pid 13)" --reason spam --note fixture
--   REPORT=$(cb reports list --json | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["id"])')
--   cb reports close "$REPORT" --resolution action_taken
--   cb token revoke "$REVOKED_ID"
--
--   kill $SERVER; wait $SERVER || true; trap - EXIT
--   # 14 has expired: prune removes its row, the trigger retires its id.
--   sleep 2
--   cb prune
--   python3 -c 'import sqlite3,sys;print("\n".join(sqlite3.connect(sys.argv[1]).iterdump()))' "$DB" > "$OUT"
--   echo "database left in $DIR"
BEGIN TRANSACTION;
CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
);
INSERT INTO "_sqlx_migrations" VALUES(1,'init','2026-11-22 15:04:10',1,X'7CF479EEBA6405ED2217A6200A8398DFBA5046E423ACCCFCE6D0F6D47E2B2BBB1D7D02C7F54FFB95FA56632A629A32FA',529576);
CREATE TABLE abuse_reports (
    id                    TEXT PRIMARY KEY,
    paste_id              TEXT NOT NULL,
    reason_code           TEXT NOT NULL,
    description           TEXT NOT NULL,
    reporter_contact      TEXT,
    created_at            INTEGER NOT NULL,
    status                TEXT NOT NULL DEFAULT 'open'
                          CHECK (status IN ('open','reviewing','closed')),
    handled_at            INTEGER,
    resolution            TEXT
);
INSERT INTO "abuse_reports" VALUES('01M81WJGVNGY0V5D3DSF4KSHTH','ExMTExMTExMTExMTExMTEw','spam','fixture report','reporter@example.org',1795359851,'closed',1795359851,'action_taken');
CREATE TABLE creation_tokens (
    id            TEXT PRIMARY KEY NOT NULL,
    verifier      BLOB UNIQUE NOT NULL,
    label         TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    last_used_on  INTEGER,
    revoked_at    INTEGER
);
INSERT INTO "creation_tokens" VALUES('01M81WJGCMV3655M9FDWASE284',X'08CE2E4B6E539A24C74F9A2248EFD18FAEAAFE3CC1F7097B341FB0C92BD11ED4','fixture active',1795359850,1795305600,NULL);
INSERT INTO "creation_tokens" VALUES('01M81WJGCTPRMWQD9FATJPRC4M',X'682D76D73669DE0581DD816FAD547A608C508F29F0FDBFBD4E86CDE6416ABF04','fixture revoked',1795359850,NULL,1795359851);
CREATE TABLE moderation_actions (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    paste_id              TEXT NOT NULL,
    action                TEXT NOT NULL,
    reason_code           TEXT,
    note                  TEXT,
    created_at            INTEGER NOT NULL
);
INSERT INTO "moderation_actions" VALUES(1,'ExMTExMTExMTExMTExMTEw','disable','spam','fixture',1795359851);
CREATE TABLE pastes (
    id                    TEXT PRIMARY KEY,
    format_version        INTEGER NOT NULL,
    public_header         BLOB NOT NULL,
    ciphertext            BLOB,
    reveal_verifier       BLOB NOT NULL,
    delete_verifier       BLOB NOT NULL,
    created_at            INTEGER NOT NULL,
    last_retrieved_at     INTEGER,
    expires_at            INTEGER,
    burn_after_retrieval  INTEGER NOT NULL DEFAULT 0
                          CHECK (burn_after_retrieval IN (0, 1)),
    consumed_at           INTEGER,
    payload_size          INTEGER NOT NULL,
    availability_status   TEXT NOT NULL DEFAULT 'active'
                          CHECK (availability_status IN ('active','disabled','deleted')),
    moderation_reason     TEXT,
    moderated_at          INTEGER,
    describe_verifier     BLOB NOT NULL
);
INSERT INTO "pastes" VALUES('EBAQEBAQEBAQEBAQEBAQEA',1,X'7B22656E76656C6F70655F666F726D6174223A312C22636F6E74656E745F666F726D6174223A2263617073756C652D66696C652D7631222C22636970686572223A224145532D3235362D47434D222C226E6F6E6365223A2241414543417751464267634943516F4C222C22686B64665F73616C74223A2245424553457851564668635947526F62484230654877227D',X'00FF666978747572652D636970686572746578742D3130',X'5820C164778D52B80A87A40BAA93E761BD44878CE84FA9A37F6A58C13ED9E8B5',X'5BEBA6432201E386367D66DCB9B160969EA442DDE5F74A9C33EABCDCEC3B0B3F',1795359851,NULL,1795964651,0,NULL,23,'active',NULL,NULL,X'8DEE9E7E7BE4721F5C074B8505B03EAB2F7B5B0B4D0C3C543DF1C8C6509808CE');
INSERT INTO "pastes" VALUES('EREREREREREREREREREREQ',1,X'7B22656E76656C6F70655F666F726D6174223A312C22636F6E74656E745F666F726D6174223A2263617073756C652D66696C652D7631222C22636970686572223A224145532D3235362D47434D222C226E6F6E6365223A2241414543417751464267634943516F4C222C22686B64665F73616C74223A2245424553457851564668635947526F62484230654877227D',NULL,X'01260F8A3F1EB7D7E47258AD43EE2B197E2AE4F8417833EC36B12D57618D4288',X'641B4B524A0D58054D1F555A285E5807D390DB00D15D6CA96B7A88C05E975A9D',1795359851,1795359851,1795964651,1,1795359851,23,'active',NULL,NULL,X'DC7898B107CBA2968F74B0BB6715F5BF5333C700A21E3E9593AABEFFD6682AA7');
INSERT INTO "pastes" VALUES('EhISEhISEhISEhISEhISEg',1,X'7B22656E76656C6F70655F666F726D6174223A312C22636F6E74656E745F666F726D6174223A2263617073756C652D66696C652D7631222C22636970686572223A224145532D3235362D47434D222C226E6F6E6365223A2241414543417751464267634943516F4C222C22686B64665F73616C74223A2245424553457851564668635947526F62484230654877222C2270617373776F7264223A7B227265717569726564223A747275652C226B6466223A2250424B4446322D484D41432D5348412D323536222C2273616C74223A22494345694979516C4A69636F4B536F724C4330754C77222C22697465726174696F6E73223A3630303030307D7D',X'00FF666978747572652D636970686572746578742D3132',X'B2114AD890C089C7850CC60FE24E5947B390B4A0BBE75A2C43CD4B16C202A841',X'7ECCF00BD030C2042424879725DC691B70F953F0DBC84A838A12CF89100A3E71',1795359851,NULL,1795964651,0,NULL,23,'active',NULL,NULL,X'AABD8C8CB92221F5D4353E2386EBB1C1BEF00E2C7E336EC6358BC6F2B9F512EE');
INSERT INTO "pastes" VALUES('ExMTExMTExMTExMTExMTEw',1,X'7B22656E76656C6F70655F666F726D6174223A312C22636F6E74656E745F666F726D6174223A2263617073756C652D66696C652D7631222C22636970686572223A224145532D3235362D47434D222C226E6F6E6365223A2241414543417751464267634943516F4C222C22686B64665F73616C74223A2245424553457851564668635947526F62484230654877227D',X'00FF666978747572652D636970686572746578742D3133',X'A710FD5DF98D38DA1144756273D6D1A705257ADC75C5036731946AD2DA750464',X'FE6FF87BB43FB0634A6BEC35B233BDA5D408F2E55CC78BE63DE8BDA399C9BA14',1795359851,NULL,1795964651,0,NULL,23,'disabled','spam',1795359851,X'9293682EFB3ED5FA580B68CD0C33A47392C4DAE0690799CE9327C778840EF46F');
CREATE TABLE retired_ids (
    id TEXT PRIMARY KEY NOT NULL
) WITHOUT ROWID;
INSERT INTO "retired_ids" VALUES('FBQUFBQUFBQUFBQUFBQUFA');
CREATE INDEX idx_pastes_expires_at ON pastes(expires_at);
CREATE INDEX idx_pastes_created_at ON pastes(created_at);
CREATE INDEX idx_pastes_last_retrieved_at ON pastes(last_retrieved_at);
CREATE INDEX idx_abuse_reports_status ON abuse_reports(status, created_at);
CREATE TRIGGER retire_deleted_paste_id
AFTER DELETE ON pastes
BEGIN
    INSERT OR IGNORE INTO retired_ids (id) VALUES (OLD.id);
END;
CREATE INDEX idx_creation_tokens_active ON creation_tokens (created_at, id)
    WHERE revoked_at IS NULL;
DELETE FROM "sqlite_sequence";
INSERT INTO "sqlite_sequence" VALUES('moderation_actions',1);
COMMIT;
