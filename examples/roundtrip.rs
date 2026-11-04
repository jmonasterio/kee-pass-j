//! Round-trip check against a real database.
//! Usage: KPJ_PW=<password> cargo run --example roundtrip -- <path-to-copy.kdbx>
//! Opens the file, records every entry, adds one entry, saves (atomic + backup into a
//! temp backup dir), reopens, and compares every original entry field by field.

use keepassj::vault::{BackupPolicy, EntryEdit, EntryView, Vault};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn snapshot(v: &Vault) -> BTreeMap<String, String> {
    let mut all: Vec<EntryView> = v.all_entries();
    // include recycle-bin entries too, via groups
    for g in v.groups() {
        if g.is_recycle_bin {
            all.extend(v.entries_in(g.id));
        }
    }
    all.into_iter()
        .map(|e| {
            let mut custom: Vec<String> = e
                .custom
                .iter()
                .map(|c| format!("{}={}:{}", c.key, c.protected, c.value.len()))
                .collect();
            custom.sort();
            let mut att = e.attachments.clone();
            att.sort();
            (
                e.id.to_string(),
                format!(
                    "{}|{}|pw{}|{}|notes{}|{:?}|{}|{:?}|{:?}|{:?}|{:?}",
                    e.title,
                    e.username,
                    e.password.len(),
                    e.url,
                    e.notes.len(),
                    e.tags,
                    e.stale,
                    custom,
                    att,
                    e.autotype_sequence,
                    e.autotype_windows
                ),
            )
        })
        .collect()
}

fn main() {
    let path = PathBuf::from(std::env::args().nth(1).expect("path argument"));
    let pw = std::env::var("KPJ_PW").expect("KPJ_PW not set");
    let backups = BackupPolicy {
        dir: std::env::temp_dir().join("keepassj").join("backups-test"),
        keep_recent: 20,
        keep_daily_days: 30,
    };

    let t = std::time::Instant::now();
    let mut v = Vault::open(&path, &pw, None).expect("open");
    println!("opened '{}' in {:?}", v.name(), t.elapsed());
    let groups = v.groups();
    let before = snapshot(&v);
    let att_bytes: usize = v
        .all_entries()
        .iter()
        .flat_map(|e| e.attachments.iter().map(move |a| (e.id, a.clone())))
        .map(|(id, a)| v.attachment(id, &a).map(|b| b.len()).unwrap_or(0))
        .sum();
    println!(
        "groups={} entries={} attachment_bytes={}",
        groups.len(),
        before.len(),
        att_bytes
    );

    let added = v
        .upsert_entry(EntryEdit {
            id: None,
            group: v.root(),
            title: "KPJ roundtrip probe".into(),
            username: "probe".into(),
            password: "Xk7mPq".into(),
            url: "https://example.com".into(),
            notes: String::new(),
            tags: vec![],
            stale: false,
            recovery_pin: "4321".into(),
            recovery_phone: String::new(),
            security_questions: String::new(),
            custom: vec![],
            autotype_sequence: None,
            autotype_windows: vec![],
        })
        .expect("upsert");

    let t = std::time::Instant::now();
    v.save(&backups).expect("save");
    println!("saved in {:?}", t.elapsed());
    drop(v);

    let v2 = Vault::open(&path, &pw, None).expect("reopen");
    let mut after = snapshot(&v2);
    let probe = after.remove(&added.to_string()).expect("probe entry missing after reopen");
    println!("probe: {probe}");
    let mut diffs = 0;
    for (k, b) in &before {
        match after.get(k) {
            Some(a) if a == b => {}
            Some(a) => {
                diffs += 1;
                println!("CHANGED {k}\n  before {b}\n  after  {a}");
            }
            None => {
                diffs += 1;
                println!("MISSING {k}");
            }
        }
    }
    for k in after.keys().filter(|k| !before.contains_key(*k)) {
        diffs += 1;
        println!("EXTRA {k}");
    }
    println!(
        "groups after={} entries compared={} diffs={}",
        v2.groups().len(),
        before.len(),
        diffs
    );
    std::process::exit(if diffs == 0 { 0 } else { 1 });
}
