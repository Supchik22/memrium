//! Application icons made on other threads. Loading an icon (an SVG drawn, a PNG decoded), putting it on its tile and
//! building its mips costs a few milliseconds; with a hundred icons in the Launchpad that was done on the thread that draws
//! the frames, and the opening animation stuttered. Now a few workers do it, and the main thread only copies the finished
//! pictures into the atlas (`State::pump_icons`).

use memrium_de::apps::{IconIndex, IconSlot};
use memrium_de::renderer::{bake_app_icon, BakedIcon, TileStyle};
use std::collections::{HashMap, HashSet};
use std::sync::{
    mpsc::{channel, Receiver, Sender},
    Arc, Mutex,
};
use std::time::Duration;

struct Job {
    id: String,
    icon: String,
    tile: Option<TileStyle>,
    /// the icon is looked up in this icon pack alone (a preview in Settings), not in the theme in use
    pack: Option<String>,
}

pub struct Done {
    pub id: String,
    /// None: no such icon
    pub icon: Option<BakedIcon>,
}

pub struct IconLoader {
    tx: Sender<Job>,
    rx: Receiver<Done>,
    /// asked for and not yet back
    pending: HashSet<String>,
}

impl IconLoader {
    /// Start the workers. They wait for the icon index (it is built off-thread too) before their first icon.
    pub fn start(index: Arc<IconSlot>) -> IconLoader {
        let (tx, jobs) = channel::<Job>();
        let (done_tx, rx) = channel::<Done>();
        let jobs = Arc::new(Mutex::new(jobs));
        // the indexes of the packs shown in Settings, each built when first asked for
        let packs: Arc<Mutex<HashMap<String, Arc<IconIndex>>>> = Default::default();
        for n in 0..3 {
            let (jobs, done_tx, index, packs) = (jobs.clone(), done_tx.clone(), index.clone(), packs.clone());
            std::thread::Builder::new()
                .name(format!("icons-{n}"))
                .spawn(move || loop {
                    // one worker takes a job at a time off the shared queue
                    let job = match jobs.lock().map(|j| j.recv()) {
                        Ok(Ok(job)) => job,
                        _ => return,
                    };
                    let index = match &job.pack {
                        Some(pack) => {
                            let Ok(mut known) = packs.lock() else { return };
                            known.entry(pack.clone()).or_insert_with(|| Arc::new(IconIndex::build_pack(pack))).clone()
                        }
                        None => loop {
                            if let Some(i) = index.get() {
                                break i;
                            }
                            std::thread::sleep(Duration::from_millis(20));
                        },
                    };
                    let icon = index.load(&job.icon, 128).map(|img| bake_app_icon(&img, job.tile));
                    if done_tx.send(Done { id: job.id, icon }).is_err() {
                        return;
                    }
                })
                .ok();
        }
        IconLoader { tx, rx, pending: HashSet::new() }
    }

    /// Ask for an icon (once: a second ask while it is on its way does nothing).
    pub fn request(&mut self, id: &str, icon: &str, tile: Option<TileStyle>) {
        if self.pending.insert(id.to_string()) {
            let _ = self.tx.send(Job { id: id.to_string(), icon: icon.to_string(), tile, pack: None });
        }
    }

    /// Like `request`, for an icon of one icon pack (`id` is its key in the atlas).
    pub fn request_pack(&mut self, id: &str, pack: &str, icon: &str, tile: Option<TileStyle>) {
        if self.pending.insert(id.to_string()) {
            let _ = self.tx.send(Job { id: id.to_string(), icon: icon.to_string(), tile, pack: Some(pack.to_string()) });
        }
    }

    /// Up to `max` finished icons.
    pub fn take(&mut self, max: usize) -> Vec<Done> {
        let mut out = Vec::new();
        while out.len() < max {
            match self.rx.try_recv() {
                Ok(d) => {
                    self.pending.remove(&d.id);
                    out.push(d);
                }
                Err(_) => break,
            }
        }
        out
    }

    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }
}
