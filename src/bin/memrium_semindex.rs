//! Builds the index of search by meaning (see `memrium_de::semsearch`): fetches the models if they are missing, then
//! indexes new and changed pictures and documents. The shell starts it on mains power while the computer is idle and
//! stops it (SIGTERM) when that ends; what was done so far is kept.

use std::sync::atomic::{AtomicBool, Ordering};

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn main() {
    // SAFETY: the handler only stores to an atomic
    unsafe {
        libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_term as *const () as libc::sighandler_t);
        // stay out of the way: the lowest CPU priority
        libc::setpriority(libc::PRIO_PROCESS, 0, 19);
    }
    if std::env::args().any(|a| a == "--fetch") {
        match memrium_de::semsearch::fetch_models() {
            Ok(()) => println!("models ready"),
            Err(e) => {
                eprintln!("memrium_semindex: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    // `--query "words"`: what the index finds (for trying it out)
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--query") {
        use memrium_de::semsearch::{index_path, Clip, Index};
        let q = args.get(i + 1).cloned().unwrap_or_default();
        let clip = Clip::load(false).map_err(|e| eprintln!("memrium_semindex: {e}")).ok();
        let qvec = clip.as_ref().and_then(|c| c.embed_text(&q).ok());
        let index = Index::load(&index_path());
        for h in index.search(qvec.as_deref(), &q, 8) {
            println!("{:.3}  {:?}  {}  {:?}", h.score, h.kind, h.path.display(), h.why);
        }
        if let (Some(c), Some(qv)) = (&clip, &qvec) {
            // every entry's raw similarity, to see how far apart they are
            for e in &index.entries {
                if e.vec.len() == qv.len() {
                    let s: f32 = e.vec.iter().zip(qv).map(|(a, b)| a * b).sum();
                    println!("   raw {s:.3}  {}", e.path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
                }
            }
            let _ = c;
        }
        return;
    }
    let t = std::time::Instant::now();
    match memrium_de::semsearch::build(&STOP) {
        Ok(n) => println!("memrium_semindex: {n} files in the index ({:.0?})", t.elapsed()),
        Err(e) => {
            memrium_de::semsearch::set_status(&format!("Error: {e}"));
            eprintln!("memrium_semindex: {e}");
            std::process::exit(1);
        }
    }
}
