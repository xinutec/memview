#[test]
fn probe() {
    let Ok(id) = std::env::var("PROBE_ID") else {
        return;
    };
    let root = console::past::projects_root();
    eprintln!("root={}", root.display());
    match console::past::transcript_of(&root, &id) {
        Some(path) => {
            let meta = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            eprintln!("found={} len={meta}", path.display());
            let page = console::past::page(&path, None);
            eprintln!("events={} from={}", page.events.len(), page.from);
            eprintln!("interactions={}", console::past::interactions(&path));
        }
        None => eprintln!("transcript_of found nothing"),
    }
}
