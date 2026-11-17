//! Live updates: watch the scanned folder with FSEvents and, for each folder that changed,
//! read it again on a background thread. The UI thread applies the results to the tree.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::scan::{self, Listing, Node, Progress, Tree};

/// Wait this long after the first change for more, so a burst becomes one update.
const SETTLE: Duration = Duration::from_millis(300);

pub enum Request {
    /// A path FSEvents reported as changed.
    Event(PathBuf),
    /// Read this folder again.
    List(String),
    /// Scan this new folder in full.
    Scan(String),
}

pub enum Patch {
    Listing { path: String, listing: Listing },
    Subtree { path: String, nodes: Vec<Node> },
}

pub struct Live {
    _watcher: RecommendedWatcher,
    tx: Sender<Request>,
    rx: Receiver<Patch>,
}

impl Live {
    /// Start watching `root` (as the tree names it). `wake` runs when patches are ready.
    pub fn start(root: String, wake: impl Fn() + Send + 'static) -> Option<Live> {
        let (tx, req_rx) = mpsc::channel::<Request>();
        let (patch_tx, rx) = mpsc::channel::<Patch>();
        let etx = tx.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                for p in ev.paths {
                    let _ = etx.send(Request::Event(p));
                }
            }
        })
        .ok()?;
        watcher.watch(Path::new(&root), RecursiveMode::Recursive).ok()?;
        std::thread::Builder::new()
            .name("spacious-live".into())
            .spawn(move || worker(root, req_rx, patch_tx, wake))
            .ok()?;
        Some(Live { _watcher: watcher, tx, rx })
    }

    pub fn request(&self, r: Request) {
        let _ = self.tx.send(r);
    }

    pub fn poll(&self) -> Vec<Patch> {
        self.rx.try_iter().collect()
    }
}

fn worker(root: String, rx: Receiver<Request>, tx: Sender<Patch>, wake: impl Fn()) {
    let root_path = PathBuf::from(&root);
    // FSEvents reports real paths (/private/tmp for /tmp): map them back to the tree's names.
    let real_root = std::fs::canonicalize(&root_path).unwrap_or_else(|_| root_path.clone());
    let skip = scan::skip_list(&root_path);
    loop {
        let Ok(first) = rx.recv() else { return };
        let mut lists = BTreeSet::new();
        let mut scans = BTreeSet::new();
        let mut add = |r: Request| match r {
            Request::Event(p) => {
                let p = match p.strip_prefix(&real_root) {
                    Ok(rel) => root_path.join(rel),
                    Err(_) if p.starts_with(&root_path) => p,
                    Err(_) => return,
                };
                if skip.iter().any(|s| p.starts_with(s)) {
                    return;
                }
                // The folder that holds the change, or the nearest one still on disk.
                let mut d = p;
                while !d.is_dir() && d.starts_with(&root_path) && d != root_path {
                    d.pop();
                }
                if d.starts_with(&root_path) {
                    lists.insert(d.to_string_lossy().into_owned());
                }
            }
            Request::List(s) => {
                lists.insert(s);
            }
            Request::Scan(s) => {
                scans.insert(s);
            }
        };
        add(first);
        let deadline = Instant::now() + SETTLE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(r) => add(r),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        for s in &scans {
            if let Some(t) = scan::scan_blocking(s, &Progress::default())
                && tx.send(Patch::Subtree { path: s.clone(), nodes: t.nodes }).is_err()
            {
                return;
            }
        }
        for l in &lists {
            // A full scan of a parent already covers it.
            if scans.iter().any(|s| l.starts_with(s.as_str())) {
                continue;
            }
            if let Some(listing) = scan::list_dir(Path::new(l), &skip)
                && tx.send(Patch::Listing { path: l.clone(), listing }).is_err()
            {
                return;
            }
        }
        wake();
    }
}

/// Apply patches to the tree. Returns follow-up requests: new folders to scan, and folders to
/// read again when a change landed somewhere the tree doesn't know yet.
pub fn apply_patches(tree: &mut Tree, patches: Vec<Patch>) -> Vec<Request> {
    let mut requests = Vec::new();
    for patch in patches {
        match patch {
            Patch::Listing { path, listing } => match tree.find(&path) {
                Some(d) if tree.node(d).is_dir => {
                    requests.extend(tree.apply_listing(d, listing).into_iter().map(Request::Scan));
                }
                // Not in the tree yet (inside a new folder): read the nearest folder that is.
                _ => requests.push(Request::List(tree.path(tree.nearest(&path)))),
            },
            Patch::Subtree { path, nodes } => match tree.find(&path) {
                Some(g) if tree.node(g).is_dir => tree.graft(g, nodes),
                _ => requests.push(Request::List(tree.path(tree.nearest(&path)))),
            },
        }
    }
    if tree.garbage > 200_000 && tree.garbage > tree.nodes.len() / 2 {
        tree.compact();
    }
    requests
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use super::*;

    /// Real FSEvents: change files on disk and wait until the live tree matches a fresh scan.
    #[test]
    fn follows_the_disk() {
        let d = std::env::temp_dir().join(format!("spacious-live-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("keep/old")).unwrap();
        fs::write(d.join("keep/old/big.mov"), vec![1u8; 2_000_000]).unwrap();
        fs::write(d.join("keep/small.txt"), vec![1u8; 10_000]).unwrap();
        let root = d.to_str().unwrap().to_owned();
        let mut tree = scan::scan_blocking(&root, &Progress::default()).unwrap();
        let live = Live::start(root.clone(), || {}).expect("watch");
        std::thread::sleep(Duration::from_millis(300));

        fs::remove_dir_all(d.join("keep/old")).unwrap(); // delete a folder
        fs::create_dir_all(d.join("new/deep/er")).unwrap(); // add nested folders
        fs::write(d.join("new/deep/er/clip.mp4"), vec![1u8; 700_000]).unwrap();
        fs::write(d.join("keep/small.txt"), vec![1u8; 90_000]).unwrap(); // grow a file

        let fresh = || scan::scan_blocking(&root, &Progress::default()).unwrap();
        let want = fresh();
        let start = Instant::now();
        loop {
            let reqs = apply_patches(&mut tree, live.poll());
            for r in reqs {
                live.request(r);
            }
            let (a, b) = (tree.node(0), want.node(0));
            if a.size == b.size && a.files == b.files && a.dirs == b.dirs {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "live tree {} B / {} files / {} dirs, disk {} B / {} files / {} dirs",
                a.size, a.files, a.dirs, b.size, b.files, b.dirs
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(tree.find(&format!("{root}/keep/old")).is_none());
        assert!(tree.find(&format!("{root}/new/deep/er/clip.mp4")).is_some());
        let _ = fs::remove_dir_all(&d);
    }
}
