//! Test helper, never shipped: a process that starts an upload and is then
//! killed, so `tests/oci_store.rs` can prove a session survives a real
//! `SIGKILL` or `TerminateProcess`, not just a dropped value.
//!
//! Usage: `oci_child <registry root>`. It opens the store, begins an upload
//! into `a/x`, appends three known frames, prints the upload id and a newline,
//! flushes, and sleeps until it is killed.

use hologram_live::oci_store::{OciStore, OpenOptions, RepoName, FRAME};
use std::io::Write;
use std::time::Duration;

/// The same bytes `tests/oci_store.rs` generates for frame `index`.
fn frame(index: u64) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&index.to_le_bytes());
    let mut out = vec![0_u8; FRAME];
    hasher.finalize_xof().fill(&mut out);
    out
}

fn main() {
    let root = std::env::args().nth(1).expect("usage: oci_child <root>");
    let options = OpenOptions {
        create: true,
        upload_max_age: Duration::from_hours(1),
    };
    let store = OciStore::open(std::path::Path::new(&root), options).expect("open");
    let repo = RepoName::parse("a/x").expect("repository name");
    let id = store.upload_begin(&repo).expect("begin");
    let mut offset = 0;
    for index in 0..3 {
        offset = store
            .upload_append(&id, offset, &frame(index))
            .expect("append");
    }
    let mut stdout = std::io::stdout();
    writeln!(stdout, "{id}").expect("print the upload id");
    stdout.flush().expect("flush");
    loop {
        std::thread::sleep(Duration::from_mins(1));
    }
}
