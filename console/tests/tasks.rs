//! The work a conversation is holding, read from the tasks service.
//!
//! Against a stub service rather than the live one: a suite that needed isis up
//! would fail on a train and pass at a desk, and would be measuring the tunnel
//! rather than the reader. The shapes here are the ones that break a naive
//! client — a session holding nothing, a service that will not answer, and a
//! second read that must not become a second request.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use console::tasks::Tasks;

/// A stub service: answers every path from a table, and counts what it was
/// asked. Returns its address and the counter, and stops when the test ends.
async fn serving(answers: Vec<(&'static str, &'static str)>) -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("addr");
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
        let answers = answers.clone();
        let counted = counted.clone();
        async move {
            counted.fetch_add(1, Ordering::SeqCst);
            let path = uri.path().to_string();
            answers
                .iter()
                .find(|(at, _)| *at == path)
                .map(|(_, body)| {
                    (
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        *body,
                    )
                        .into_response()
                })
                .unwrap_or_else(|| axum::http::StatusCode::NOT_FOUND.into_response())
        }
    });
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (address, asked)
}

use axum::response::IntoResponse;

/// A reader pointed at `address`.
///
/// ⚠ **Told where to look, rather than through `TASKS_URL`.** These tests run in
/// parallel in one process, and an environment variable is shared by all of
/// them: setting it per test had each stub answering somebody else's reader.
fn reading(address: SocketAddr) -> Tasks {
    // No crypto provider to install here: `Tasks::at` does it, because a type
    // that panics unless the caller remembered something is a trap — see the
    // note there.
    //
    // ⚠ **Pointed at a store that is not there.** Left at its default this
    // counts the leftovers in the real `~/.claude/tasks` of whoever is running
    // the suite, so every assertion about which sessions appear would depend on
    // the machine — eleven extra rows on this Mac today, none on a fresh one.
    Tasks::at(format!("http://{address}")).counting(scratch("empty"))
}

/// A directory under the temp dir that belongs to this call and to nothing else.
///
/// ⚠ **Unique per CALL, not per name and not per process, and both of those were
/// tried.** The failure is always the same one: `remove_dir_all` then
/// `create_dir_all` is not atomic, so one caller removing the directory between
/// another's remove and create fails as `AlreadyExists` — which reads as a
/// defect in whatever was changed last rather than as a race in the harness.
///
/// Naming it after the caller fixed nothing, because [`reading`] passes the same
/// name from every test in this file and they run in parallel threads: thirteen
/// of them thrashing one path. Adding the process id fixed the *other* race —
/// two suites at once, a neighbour's gate against this one — and left that. A
/// counter is what actually settles it: nothing is ever shared, so nothing has
/// to be emptied, and there is no window to lose.
fn scratch(what: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mine = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "console-strays-{what}-{}-{mine}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

/// A built-in store holding `count` task files for `session`, plus the two
/// dotfiles the CLI owns and that are not tasks.
fn leaving(store: &std::path::Path, session: &str, count: usize) {
    let dir = store.join(session);
    std::fs::create_dir_all(&dir).expect("session dir");
    std::fs::write(dir.join(".lock"), "").expect("lock");
    std::fs::write(dir.join(".highwatermark"), "7").expect("highwatermark");
    for id in 0..count {
        std::fs::write(dir.join(format!("{id}.json")), "{}").expect("task");
    }
}

/// The holder rows, as the live service gives them: sessions first by open
/// descending, then the person, then the pile — and the pile with no `id` at
/// all, because it is not anybody.
const HOLDERS: &str = r#"[{"kind":"session","id":"alive","name":"health","open":3,"total":47},
                          {"kind":"session","id":"cleared","open":0,"total":9},
                          {"kind":"session","id":"idle","open":0,"total":0},
                          {"kind":"person","id":"pippijn","name":"Pippijn","open":1,"total":12},
                          {"kind":"nobody","name":"nobody","open":23,"total":26}]"#;

#[tokio::test]
async fn a_session_that_was_never_handed_anything_is_absent_rather_than_zero() {
    // The rule the client draws by: no work is not an empty list, and a card
    // saying `0` would be a claim where nothing is the truth.
    let (address, _) = serving(vec![("/api/holders", HOLDERS)]).await;
    let swept = reading(address).sweep().await;
    assert_eq!(swept.sessions.get("alive").map(|c| c.open), Some(3));
    assert!(
        !swept.sessions.contains_key("idle"),
        "a session that has never held anything"
    );
}

#[tokio::test]
async fn a_session_that_finished_its_list_still_gets_a_row() {
    // ⚠ The case `open > 0` used to hide, and the reason the rule is now keyed
    // on the total: `0/9` is a session that cleared its plate, which is a
    // different fact from never having been given one — and the better of the
    // two to be able to see.
    let (address, _) = serving(vec![("/api/holders", HOLDERS)]).await;
    let swept = reading(address).sweep().await;
    let cleared = swept.sessions.get("cleared").expect("a finished list");
    assert_eq!((cleared.open, cleared.total), (0, 9));
}

#[tokio::test]
async fn the_total_is_what_is_assigned_now_and_never_smaller_than_the_open() {
    let (address, _) = serving(vec![("/api/holders", HOLDERS)]).await;
    let swept = reading(address).sweep().await;
    for (id, count) in &swept.sessions {
        assert!(
            count.total >= count.open,
            "{id} holds more open than it has"
        );
    }
}

#[tokio::test]
async fn the_person_and_the_pile_are_kept_in_the_order_they_came() {
    // Not sessions, and not thrown away: a card reading `3/47` means one thing
    // beside "23 are in the pile" and another without it. The pile is on no
    // card, because it belongs to no conversation.
    let (address, _) = serving(vec![("/api/holders", HOLDERS)]).await;
    let swept = reading(address).sweep().await;
    let named: Vec<_> = swept.elsewhere.iter().map(|held| &*held.name).collect();
    assert_eq!(
        named,
        ["Pippijn", "nobody"],
        "the service decides the order"
    );
    assert_eq!(
        (swept.elsewhere[1].open, swept.elsewhere[1].total),
        (23, 26)
    );
}

#[tokio::test]
async fn what_is_left_in_the_store_this_replaced_is_counted_beside_it() {
    // Every file there is re-sent to its session 1.75 times per message, bodies
    // and all. A number on the card says either "migrated but never deleted" or
    // "still filing work into the expensive store".
    let store = scratch("left-behind");
    leaving(&store, "alive", 3);
    let (address, _) = serving(vec![("/api/holders", HOLDERS)]).await;
    let swept = Tasks::at(format!("http://{address}"))
        .counting(&store)
        .sweep()
        .await;
    assert_eq!(
        swept.sessions["alive"].stray, 3,
        "the dotfiles are not tasks"
    );
    // ⚠ The one that has cleaned up says nothing rather than zero, so the number
    // on a card only ever means there is something to do about it.
    assert_eq!(swept.sessions["cleared"].stray, 0);
}

#[tokio::test]
async fn a_session_with_only_leftovers_gets_a_row_the_service_cannot_give_it() {
    // The sharpest case: work being filed into the store nothing reads, by a
    // conversation with nothing in the one everybody does. It has no holder row
    // by definition, so a sweep of holders alone would never show it.
    let store = scratch("only-leftovers");
    leaving(&store, "astray", 12);
    let (address, _) = serving(vec![("/api/holders", HOLDERS)]).await;
    let swept = Tasks::at(format!("http://{address}"))
        .counting(&store)
        .sweep()
        .await;
    let stranded = swept.sessions.get("astray").expect("a row of its own");
    assert_eq!((stranded.open, stranded.total, stranded.stray), (0, 0, 12));
}

#[tokio::test]
async fn a_second_read_inside_the_ttl_is_not_a_second_request() {
    // The whole reason a per-poll read is affordable: the front page polls every
    // five seconds, per client, and the service is on another machine.
    let (address, asked) = serving(vec![("/api/holders", HOLDERS)]).await;
    let tasks = reading(address);
    assert_eq!(tasks.sweep().await.sessions.len(), 2);
    assert_eq!(tasks.sweep().await.sessions.len(), 2);
    assert_eq!(asked.load(Ordering::SeqCst), 1, "the cache was skipped");
}

#[tokio::test]
async fn a_service_that_will_not_answer_serves_the_last_known_counts() {
    // Stale beats blocking, and both beat failing. A console left running
    // through a reboot of isis shows the list it had, not an empty page.
    let (address, _) = serving(vec![("/api/holders", HOLDERS)]).await;
    let tasks = reading(address);
    assert_eq!(
        tasks.sweep().await.sessions.get("alive").map(|c| c.open),
        Some(3)
    );

    // Point it at a port with nothing behind it and expire the cache by hand is
    // not possible from here, so the failure is proven the other way: a reader
    // that has never had an answer returns nothing rather than raising.
    let dead = reading("127.0.0.1:1".parse().expect("addr"));
    assert!(
        dead.sweep().await.sessions.is_empty(),
        "no answer, and no panic"
    );
}

#[tokio::test]
async fn a_list_carries_what_a_row_needs_and_not_the_prose() {
    let body = r#"[{"id":631,"repo":"memview","subject":"A slash command becomes prose","status":"open","assignee":{"kind":"nobody"},"detailed":true,"created_at":"2026-08-08T10:00:00Z","updated_at":"2026-08-08T10:00:00Z"},
                   {"id":97,"repo":"memview","subject":"Finished already","status":"done","assignee":{"kind":"nobody"},"detailed":false,"created_at":"2026-08-08T10:00:00Z","updated_at":"2026-08-08T10:00:00Z"}]"#;
    let (address, _) = serving(vec![("/api/tasks", body)]).await;
    let listed = reading(address).listed("whoever").await;
    assert_eq!(listed.len(), 2);
    // ⚠ The number arrives as a JSON number and is a string everywhere above
    // this — it is what a session calls a task in its own prose, `#631`.
    assert_eq!(listed[0].id, "631");
    assert_eq!(listed[0].subject, "A slash command becomes prose");
    assert_eq!(listed[0].status, "open");
    assert!(listed[0].detailed, "there is prose worth opening");
    // The service's own words, not a boolean of ours: `doing` is a third state
    // and the client sorts on it.
    assert_eq!(listed[1].status, "done");
    assert!(!listed[1].detailed);
}

#[tokio::test]
async fn a_rank_is_carried_through_and_no_rank_stays_absent() {
    // The order is the service's — `repo::list` sorts, and P3 is BELOW the
    // untriaged, which is why the ranked row here is the last one. Nothing on
    // this side may reorder them, so the ranked task is deliberately not first.
    let body = r#"[{"id":748,"repo":"memview","subject":"No rank at all","status":"open","assignee":{"kind":"nobody"},"detailed":true,"created_at":"2026-08-11T10:00:00Z","updated_at":"2026-08-11T10:00:00Z"},
                   {"id":96,"repo":"memview","subject":"When there is room","status":"open","assignee":{"kind":"nobody"},"detailed":true,"priority":"P3","created_at":"2026-08-11T10:00:00Z","updated_at":"2026-08-11T10:00:00Z"}]"#;
    let (address, _) = serving(vec![("/api/tasks", body)]).await;
    let listed = reading(address).listed("whoever").await;
    assert_eq!(listed[0].id, "748", "the answer's order, unsorted");
    assert_eq!(listed[0].priority, None);
    assert_eq!(listed[1].priority.as_deref(), Some("P3"));

    // ⚠ Absent on the way out as well as in. A `null` here would be a rank a
    // client could draw a placeholder for, on the 98% of rows that have none.
    let out = serde_json::to_string(&listed[0]).expect("serialisable");
    assert!(
        !out.contains("priority"),
        "no empty rank on the wire: {out}"
    );
}

#[tokio::test]
async fn a_task_with_no_prose_offers_none() {
    // Offering to open an empty sheet is worse than not offering.
    let empty = r#"{"id":97,"subject":"x","status":"open","assignee":{"kind":"nobody"},"detailed":false,"created_at":"2026-08-08T10:00:00Z","updated_at":"2026-08-08T10:00:00Z","body":"   ","body_html":"","events":[]}"#;
    let (address, _) = serving(vec![("/api/tasks/97", empty)]).await;
    assert_eq!(reading(address).detail("97").await, None);
}

#[tokio::test]
async fn a_task_with_prose_returns_the_markdown_not_the_html() {
    // Both are sent. The console renders markdown itself — see `rendered.ts` —
    // and taking the HTML would put content outside that renderer's rules.
    // ⚠ `r###"…"###`, not one or two hashes. The body starts `"## Why`, and
    // that sequence closes BOTH `r#"…"#` and `r##"…"##`. Same trap as the
    // `## Context Usage` fixture in `console/tests/past.rs`.
    let full = r###"{"id":98,"subject":"x","status":"open","assignee":{"kind":"nobody"},"detailed":true,"created_at":"2026-08-08T10:00:00Z","updated_at":"2026-08-08T10:00:00Z","body":"## Why\n\nBecause.","body_html":"<h2>Why</h2>","events":[]}"###;
    let (address, _) = serving(vec![("/api/tasks/98", full)]).await;
    let said = reading(address).detail("98").await.expect("prose");
    assert!(said.starts_with("## Why"), "markdown, not html: {said}");
}

#[tokio::test]
async fn a_task_that_is_not_there_is_not_an_error() {
    let (address, _) = serving(vec![]).await;
    assert_eq!(reading(address).detail("404").await, None);
}
