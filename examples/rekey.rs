//! Change the master password of a database.
//! Usage: KPJ_PW=<old> KPJ_NEW_PW=<new> cargo run --example rekey -- <path.kdbx>
//! The pre-change file is kept as a backup under %TEMP%\keepassj\backups-rekey.

use keepassj::vault::{BackupPolicy, Vault};
use std::path::PathBuf;

fn main() {
    let path = PathBuf::from(std::env::args().nth(1).expect("path argument"));
    let old = std::env::var("KPJ_PW").expect("KPJ_PW not set");
    let new = std::env::var("KPJ_NEW_PW").expect("KPJ_NEW_PW not set");
    let backups = BackupPolicy {
        dir: std::env::temp_dir().join("keepassj").join("backups-rekey"),
        keep_recent: 20,
        keep_daily_days: 30,
    };
    let mut v = Vault::open(&path, &old, None).expect("open with old password");
    let count = v.all_entries().len();
    v.change_key(&new, None).expect("change key");
    v.save(&backups).expect("save");
    drop(v);
    let v = Vault::open(&path, &new, None).expect("reopen with new password");
    assert_eq!(v.all_entries().len(), count);
    assert!(Vault::open(&path, &old, None).is_err(), "old password still opens the file");
    println!("rekeyed {} ({} entries); new password opens it, old one does not", path.display(), count);
}
