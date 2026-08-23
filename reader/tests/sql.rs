//! What the SQL reader names, and — more importantly — what it refuses to.

use reader::sql::read;

/// The measurement that shaped the grammar: a function is not a table.
///
/// ⚠ **This is the test the whole module exists for.** A regular expression for
/// `FROM (\w+)` answers `datetime` here, and a fabricated subject is worse than
/// a missing one: it corrupts every count downstream and nothing says so.
#[test]
fn a_function_call_is_not_a_table() {
    let got = read("SELECT ts FROM datetime(ts, 'unixepoch')");
    assert!(got.reads.is_empty(), "named a table: {:?}", got.reads);
}

#[test]
fn select_reads_its_from_and_its_joins() {
    let got = read("SELECT a.x FROM audio_segments a JOIN transcript_segments t ON t.id = a.id");
    assert_eq!(got.reads.get("audio_segments"), Some(&1));
    assert_eq!(got.reads.get("transcript_segments"), Some(&1));
    assert!(got.writes.is_empty());
}

/// The line `Direction` exists for.
#[test]
fn delete_from_changes_the_table_select_from_only_reads_it() {
    let deleted = read("DELETE FROM check_result WHERE id = 3");
    assert_eq!(deleted.writes.get("check_result"), Some(&1));
    assert!(deleted.reads.is_empty(), "a deletion is not a read");

    let selected = read("SELECT * FROM check_result");
    assert_eq!(selected.reads.get("check_result"), Some(&1));
    assert!(selected.writes.is_empty());
}

/// A join under a changing verb is consulted, not targeted.
#[test]
fn a_join_is_read_even_under_delete() {
    let got = read("DELETE a FROM sources a JOIN report b ON a.id = b.id");
    assert_eq!(got.writes.get("sources"), Some(&1));
    assert_eq!(got.reads.get("report"), Some(&1));
}

#[test]
fn insert_select_writes_the_target_and_reads_the_source() {
    let got = read("INSERT INTO report (id) SELECT id FROM irc_messages");
    assert_eq!(got.writes.get("report"), Some(&1));
    assert_eq!(got.reads.get("irc_messages"), Some(&1));
}

#[test]
fn update_changes_its_target() {
    let got = read("UPDATE recall SET done = 1 WHERE id = 2");
    assert_eq!(got.writes.get("recall"), Some(&1));
}

#[test]
fn ddl_names_the_table_it_alters() {
    assert_eq!(
        read("DROP TABLE IF EXISTS scratch").writes.get("scratch"),
        Some(&1)
    );
    assert_eq!(
        read("TRUNCATE TABLE sessions").writes.get("sessions"),
        Some(&1)
    );
    assert_eq!(
        read("ALTER TABLE report ADD COLUMN n INT")
            .writes
            .get("report"),
        Some(&1)
    );
}

#[test]
fn backticks_and_qualified_names_survive() {
    let got = read("SELECT * FROM `health`.`heart_rate_intraday`");
    assert_eq!(got.reads.get("health.heart_rate_intraday"), Some(&1));
}

/// A subquery names no table of its own; the tables inside it do.
#[test]
fn a_subquery_contributes_only_its_own_tables() {
    let got = read("SELECT * FROM (SELECT id FROM audio_segments) x");
    assert_eq!(got.reads.get("audio_segments"), Some(&1));
    assert_eq!(
        got.reads.len(),
        1,
        "invented a table for the subquery: {:?}",
        got.reads
    );
}

/// Measured absent from this corpus, read anyway — the cost of missing one is a
/// write nobody sees.
#[test]
fn the_clauses_that_name_a_file_still_name_one() {
    let out = read("SELECT * FROM report INTO OUTFILE '/tmp/report.tsv'");
    assert_eq!(out.uses.len(), 1);
    assert_eq!(out.uses[0].path, "/tmp/report.tsv");
    assert!(out.uses[0].write);

    let dot = read(".output /tmp/dump.txt");
    assert_eq!(dot.uses.len(), 1);
    assert!(dot.uses[0].write);
}

/// `.output stdout` names a stream. Recording it as a file would invent one.
#[test]
fn a_stream_is_not_a_file() {
    assert!(read(".output stdout").uses.is_empty());
}

#[test]
fn a_dot_command_with_no_subject_names_nothing() {
    let got = read(".tables");
    assert!(got.uses.is_empty());
    assert!(got.reads.is_empty());
}

/// Never rejects: what it cannot read shows up as nothing understood, not as a
/// parse error — the rule every carried reader here is grown by.
#[test]
fn nonsense_is_read_as_nothing_rather_than_refused() {
    let got = read("!!! not sql at all ###");
    assert!(got.is_empty());
}

/// `CREATE TABLE IF NOT EXISTS x` names `x`, never `IF`.
#[test]
fn if_not_exists_is_a_guard_not_a_table() {
    let got = read("CREATE TABLE IF NOT EXISTS scratch (id INT)");
    assert_eq!(got.writes.get("scratch"), Some(&1));
    assert_eq!(
        got.writes.get("IF"),
        None,
        "invented a table: {:?}",
        got.writes
    );
}

/// A dot command ends at its line — the statement after it is still a statement.
///
/// ⚠ **This was 50 corpus scripts read as nothing.** `.mode column` consumed the
/// query on the next line, so a script that opened with any sqlite directive
/// contributed no tables at all.
#[test]
fn a_dot_command_does_not_swallow_the_next_line() {
    let got = read(".mode column\nSELECT n FROM corrections");
    assert_eq!(got.reads.get("corrections"), Some(&1));
}

#[test]
fn several_directives_then_a_query() {
    let got = read(".headers on\n.mode column\nselect provenance from report");
    assert_eq!(got.reads.get("report"), Some(&1));
}
