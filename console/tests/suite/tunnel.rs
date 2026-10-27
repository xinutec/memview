//! The phone's way in: what ssh's output says, and the outages it adds up to.
//! The lines are ssh's own, captured from OpenSSH 10.3 against isis.

use std::path::PathBuf;

use console::tunnel::{Dial, Heard, Outage, WayIn, heard};

#[test]
fn isis_holding_the_address_is_the_way_in_up() {
    // Printed once isis has answered every forward asked for. Before it, the
    // connection exists but nothing reaches the console.
    assert_eq!(
        heard("debug1: forwarding_success: all expected forwarding replies received"),
        Heard::Up
    );
    assert_eq!(
        heard(
            "debug1: Remote connections from 10.100.0.2:8097 forwarded to local address 127.0.0.1:8097"
        ),
        Heard::Chatter,
        "asking is not the same as being granted"
    );
}

#[test]
fn what_ssh_says_going_wrong_is_the_cause() {
    for said in [
        "ssh: connect to host isis.xinutec.org port 22: Operation timed out",
        "ssh: connect to host isis.xinutec.org port 22: Connection refused",
        "Connection to isis.xinutec.org closed by remote host.",
        "Error: remote port forwarding failed for listen port 8097",
        "Timeout, server isis.xinutec.org not responding.",
    ] {
        assert_eq!(heard(said), Heard::Trouble(said.to_string()));
    }
}

#[test]
fn what_ssh_says_going_right_is_not_trouble() {
    for said in [
        "Authenticated to 10.100.0.2 ([10.100.0.2]:22) using \"publickey\".",
        "Transferred: sent 3640, received 3760 bytes, in 5.8 seconds",
        "Bytes per second: sent 624.6, received 645.2",
        "debug1: channel 0: new forwarded-tcpip",
        "",
    ] {
        assert_eq!(heard(said), Heard::Chatter, "{said:?}");
    }
}

#[test]
fn an_outage_runs_from_the_drop_to_isis_answering_and_names_the_first_cause() {
    // A reboot of isis: closed by isis, then refused while sshd stopped, then
    // timing out while it was down. Why it went down is the first of these.
    let mut way = WayIn::default();
    way.up(1_000);
    way.trouble("Connection to isis.xinutec.org closed by remote host.");
    way.ended(2_000, "ssh exit status: 255");
    way.trouble("ssh: connect to host isis.xinutec.org port 22: Connection refused");
    way.ended(3_000, "ssh exit status: 255");
    way.trouble("ssh: connect to host isis.xinutec.org port 22: Operation timed out");
    way.ended(4_000, "ssh exit status: 255");
    assert_eq!(way.down_since, Some(2_000));
    assert_eq!(way.up_since, None);

    let ended = way.up(5_000);
    let outage = Outage {
        from: 2_000,
        to: 5_000,
        cause: "Connection to isis.xinutec.org closed by remote host.".to_string(),
    };
    assert_eq!(ended, Some(outage.clone()));
    assert_eq!(way.outages, vec![outage]);
    assert_eq!(
        (way.up_since, way.down_since, way.cause),
        (Some(5_000), None, None)
    );
}

#[test]
fn an_ssh_that_ends_without_a_word_is_named_by_how_it_ended() {
    let mut way = WayIn::default();
    way.up(1_000);
    way.ended(2_000, "ssh signal: 9");
    assert_eq!(
        way.up(3_000).map(|outage| outage.cause),
        Some("ssh signal: 9".to_string())
    );
}

#[test]
fn the_first_dial_coming_up_is_no_outage() {
    // A console starting is not the way in going down.
    let mut way = WayIn::default();
    assert_eq!(way.up(1_000), None);
    assert!(way.outages.is_empty());
}

#[test]
fn a_first_dial_that_fails_is_an_outage_from_its_failure() {
    let mut way = WayIn::default();
    way.trouble("ssh: connect to host isis.xinutec.org port 22: Operation timed out");
    way.ended(1_000, "ssh exit status: 255");
    assert_eq!(
        way.up(9_000).map(|outage| (outage.from, outage.to)),
        Some((1_000, 9_000))
    );
}

#[test]
fn trouble_while_up_is_forgotten_once_it_is_up_again() {
    // A word that ended nothing must not name the next, unrelated outage.
    let mut way = WayIn::default();
    way.up(1_000);
    way.trouble("Warning: something passing");
    way.ended(2_000, "ssh exit status: 255");
    way.up(3_000);
    way.ended(4_000, "ssh signal: 15");
    assert_eq!(
        way.up(5_000).map(|outage| outage.cause),
        Some("ssh signal: 15".to_string())
    );
}

#[test]
fn while_up_the_last_word_before_the_drop_is_the_cause() {
    // A warning hours before says nothing about a drop; what ssh said as it gave
    // up does.
    let mut way = WayIn::default();
    way.up(1_000);
    way.trouble("Warning: something passing");
    way.trouble("Timeout, server isis.xinutec.org not responding.");
    way.ended(2_000, "ssh exit status: 255");
    assert_eq!(
        way.up(3_000).map(|outage| outage.cause),
        Some("Timeout, server isis.xinutec.org not responding.".to_string())
    );
}

#[test]
fn only_the_newest_outages_are_kept() {
    let mut way = WayIn::default();
    for n in 0..30 {
        way.ended(n * 10, "ssh exit status: 255");
        way.up(n * 10 + 5);
    }
    assert_eq!(way.outages.len(), 20);
    assert_eq!(way.outages.first().map(|outage| outage.from), Some(100));
    assert_eq!(way.outages.last().map(|outage| outage.from), Some(290));
}

#[test]
fn a_restart_keeps_the_outages_and_forgets_the_dial() {
    // The last console's ssh ended with it: whatever it said about being up is
    // no longer true.
    let mut way = WayIn::default();
    way.ended(1_000, "ssh exit status: 255");
    way.up(2_000);
    let outages = way.outages.clone();
    let restarted = way.restarted();
    assert_eq!(restarted.outages, outages);
    assert_eq!(
        (restarted.up_since, restarted.down_since, restarted.cause),
        (None, None, None)
    );
}

#[test]
fn the_record_reads_back_what_it_wrote() {
    let mut way = WayIn::default();
    way.ended(1_000, "ssh exit status: 255");
    way.up(2_000);
    let text = serde_json::to_string(&way).expect("written");
    assert_eq!(serde_json::from_str::<WayIn>(&text).expect("read"), way);
}

#[test]
fn ssh_is_asked_for_exactly_the_forward_and_gives_up_on_a_silent_host() {
    let dial = Dial {
        program: "ssh".to_string(),
        host: "someone@isis.xinutec.org".to_string(),
        key: PathBuf::from("/keys/console-tunnel"),
        listen: "10.100.0.2:8097".to_string(),
        local: "127.0.0.1:8097".to_string(),
        record: PathBuf::from("/tmp/way-in.json"),
    };
    let args = dial.args();
    let has = |pair: [&str; 2]| args.windows(2).any(|w| w == pair);
    assert!(has(["-R", "10.100.0.2:8097:127.0.0.1:8097"]));
    assert!(has(["-i", "/keys/console-tunnel"]));
    // Without it a host that does not answer holds a dial for macOS's 75 seconds.
    assert!(has(["-o", "ConnectTimeout=10"]));
    // Connected but not listening would look up and pass nothing.
    assert!(has(["-o", "ExitOnForwardFailure=yes"]));
    assert!(has(["-o", "BatchMode=yes"]));
    assert!(
        args.contains(&"-v".to_string()),
        "the up line is a debug line"
    );
    assert_eq!(
        args.last().map(String::as_str),
        Some("someone@isis.xinutec.org")
    );
}
