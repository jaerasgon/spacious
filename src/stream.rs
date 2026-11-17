//! Streaming scan: background threads read folders in parallel and send each one to the UI as
//! soon as it's read, so the map fills in while the scan runs.
//!
//! Each folder gets a numeric id when its parent is read. The UI keeps id → node index, appends
//! a folder's children to the arena when its message arrives, and adds its totals to every
//! folder above. Nodes don't move during a scan; `finish` sorts and compacts at the end.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use rayon::prelude::*;

use crate::scan::{NONE, Node, Progress, Tree, skip_list};
use crate::util::Cat;

/// One folder, as read from disk.
pub struct DirMsg {
    pub id: u32,
    pub files: Vec<(Box<str>, u64, i64)>,
    /// Subfolders with their mtime and the id their own message will carry.
    pub dirs: Vec<(Box<str>, i64, u32)>,
    pub unreadable: bool,
}

pub struct ScanJob {
    pub root: String,
    pub progress: Arc<Progress>,
    pub started: Instant,
    rx: mpsc::Receiver<DirMsg>,
    done: Arc<AtomicBool>,
}

impl ScanJob {
    pub fn cancel(&self) {
        self.progress.cancel.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.progress.cancel.load(Ordering::Relaxed)
    }
    /// The walkers have stopped. Read this before draining: once it's true and the channel is
    /// empty, every message has arrived.
    pub fn done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
    pub fn try_recv(&self) -> Option<DirMsg> {
        self.rx.try_recv().ok()
    }
}

struct Ctx {
    p: Arc<Progress>,
    tx: mpsc::Sender<DirMsg>,
    next_id: AtomicU32,
    links: Vec<Mutex<HashSet<(u64, u64)>>>,
    skip: Vec<PathBuf>,
    wake: Box<dyn Fn() + Send + Sync>,
    started: Instant,
    last_wake_ms: AtomicU64,
}

impl Ctx {
    fn cancelled(&self) -> bool {
        self.p.cancel.load(Ordering::Relaxed)
    }
    fn first_link(&self, dev: u64, ino: u64) -> bool {
        self.links[(ino as usize) % self.links.len()].lock().unwrap().insert((dev, ino))
    }
    /// Wake the UI at most every 50 ms.
    fn nudge(&self) {
        let now = self.started.elapsed().as_millis() as u64;
        let last = self.last_wake_ms.load(Ordering::Relaxed);
        if now >= last + 50 && self.last_wake_ms.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            (self.wake)();
        }
    }
}

/// Start a streaming scan of `root`. The tree it fills starts from `new_tree(root)`.
pub fn start(root: String, wake: impl Fn() + Send + Sync + 'static) -> ScanJob {
    let progress = Arc::new(Progress::default());
    let (tx, rx) = mpsc::channel();
    let done = Arc::new(AtomicBool::new(false));
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
    let (p, d, r, w) = (progress.clone(), done.clone(), root.clone(), wake.clone());
    std::thread::Builder::new()
        .name("spacious-scan".into())
        .spawn(move || {
            let root_path = PathBuf::from(&r);
            let w2 = w.clone();
            let ctx = Ctx {
                p,
                tx,
                next_id: AtomicU32::new(1),
                links: (0..64).map(|_| Mutex::new(HashSet::new())).collect(),
                skip: skip_list(&root_path),
                wake: Box::new(move || w2()),
                started: Instant::now(),
                last_wake_ms: AtomicU64::new(0),
            };
            let threads = std::thread::available_parallelism().map(|n| n.get() * 2).unwrap_or(8).clamp(4, 32);
            if let Ok(pool) = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .stack_size(8 * 1024 * 1024)
                .thread_name(|i| format!("spacious-walk-{i}"))
                .build()
            {
                pool.install(|| walk(&root_path, 0, &ctx));
            }
            drop(ctx);
            d.store(true, Ordering::Release);
            w();
        })
        .expect("spawn scan thread");
    ScanJob { root, progress, started: Instant::now(), rx, done }
}

fn walk(path: &Path, id: u32, ctx: &Ctx) {
    if ctx.cancelled() {
        return;
    }
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) => {
            ctx.p.unreadable.fetch_add(1, Ordering::Relaxed);
            let _ = ctx.tx.send(DirMsg { id, files: Vec::new(), dirs: Vec::new(), unreadable: true });
            return;
        }
    };
    if let Ok(mut cur) = ctx.p.current.try_lock() {
        cur.clear();
        cur.push_str(&path.to_string_lossy());
    }
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut sub = Vec::new();
    let mut bytes = 0u64;
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let name: Box<str> = entry.file_name().to_string_lossy().into();
        if ft.is_dir() {
            let p = entry.path();
            if ctx.skip.iter().any(|s| s == &p) {
                continue;
            }
            let mtime = entry.metadata().map(|m| m.mtime()).unwrap_or(0);
            let cid = ctx.next_id.fetch_add(1, Ordering::Relaxed);
            dirs.push((name, mtime, cid));
            sub.push((p, cid));
        } else if ft.is_file() {
            let Ok(md) = entry.metadata() else { continue };
            let mut size = md.blocks() * 512;
            if md.nlink() > 1 && !ctx.first_link(md.dev(), md.ino()) {
                size = 0;
            }
            bytes += size;
            files.push((name, size, md.mtime()));
        }
    }
    ctx.p.files.fetch_add(files.len() as u64, Ordering::Relaxed);
    ctx.p.bytes.fetch_add(bytes, Ordering::Relaxed);
    ctx.p.dirs.fetch_add(1, Ordering::Relaxed);
    // Send before descending, so a folder always arrives before its subfolders.
    if ctx.tx.send(DirMsg { id, files, dirs, unreadable: false }).is_err() {
        return;
    }
    ctx.nudge();
    sub.into_par_iter().for_each(|(p, cid)| walk(&p, cid, ctx));
}

/// A tree holding just the root folder, ready to be filled by a streaming scan.
pub fn new_tree(root: &str) -> Tree {
    let mtime = fs::metadata(root).map(|m| m.mtime()).unwrap_or(0);
    Tree {
        nodes: vec![Node { name: root.into(), mtime, is_dir: true, ..Node::dead() }],
        root_path: root.to_owned(),
        unreadable: 0,
        elapsed: Duration::ZERO,
        garbage: 0,
    }
}

/// Applies scan messages to a tree on the UI thread.
#[derive(Default)]
pub struct Builder {
    /// Folder id → node index.
    map: Vec<u32>,
    /// Bytes per file type under each folder, so folder colours are right while scanning.
    by_cat: HashMap<u32, [u64; Cat::ALL.len()]>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder { map: vec![0], by_cat: HashMap::new() }
    }

    pub fn apply(&mut self, tree: &mut Tree, m: DirMsg) {
        let Some(&d) = self.map.get(m.id as usize) else { return };
        if d == NONE {
            return;
        }
        if m.unreadable {
            tree.nodes[d as usize].unreadable = true;
            tree.unreadable += 1;
            return;
        }
        let first = tree.nodes.len() as u32;
        let n = (m.files.len() + m.dirs.len()) as u32;
        let (mut size, nfiles, ndirs) = (0u64, m.files.len() as u64, m.dirs.len() as u32);
        let mut cats = [0u64; Cat::ALL.len()];
        for (name, s, mtime) in m.files {
            let cat = Cat::of(&name);
            size += s;
            cats[cat as usize] += s;
            tree.nodes.push(Node { name, parent: d, size: s, files: 1, mtime, cat, ..Node::dead() });
        }
        if let Some(&max) = m.dirs.iter().map(|(_, _, c)| c).max()
            && self.map.len() <= max as usize
        {
            self.map.resize(max as usize + 1, NONE);
        }
        for (name, mtime, cid) in m.dirs {
            let cat = if name.ends_with(".app") { Cat::Apps } else { Cat::Other };
            self.map[cid as usize] = tree.nodes.len() as u32;
            tree.nodes.push(Node { name, parent: d, mtime, is_dir: true, cat, ..Node::dead() });
        }
        {
            let node = &mut tree.nodes[d as usize];
            node.first_child = if n > 0 { first } else { NONE };
            node.n_children = n;
            node.cap = n;
        }
        // Add the new totals to this folder and every folder above it.
        let any = cats.iter().any(|&c| c > 0);
        let mut cur = d;
        while cur != NONE {
            let node = &mut tree.nodes[cur as usize];
            node.size += size;
            node.files += nfiles;
            node.dirs += ndirs;
            if any {
                let acc = self.by_cat.entry(cur).or_insert([0; Cat::ALL.len()]);
                for (a, b) in acc.iter_mut().zip(cats) {
                    *a += b;
                }
                if !node.name.ends_with(".app")
                    && let Some((k, _)) = acc.iter().enumerate().max_by_key(|(_, v)| **v)
                {
                    node.cat = Cat::ALL[k];
                }
            }
            cur = node.parent;
        }
    }
}

/// Sort and compact a tree once its scan has ended. Node indices change.
pub fn finish(tree: &mut Tree, elapsed: Duration) {
    tree.compact();
    tree.elapsed = elapsed;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::scan_blocking;

    #[test]
    fn streamed_tree_matches_full_scan() {
        let d = std::env::temp_dir().join(format!("spacious-stream-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for i in 0..20 {
            let sub = d.join(format!("d{i}/inner{}", i % 3));
            fs::create_dir_all(&sub).unwrap();
            for j in 0..50 {
                fs::write(sub.join(format!("f{j}.{}", if j % 2 == 0 { "mov" } else { "txt" })), vec![1u8; 4096 * (j + 1)])
                    .unwrap();
            }
        }
        fs::create_dir_all(d.join("empty")).unwrap();
        let root = d.to_str().unwrap().to_owned();

        let job = start(root.clone(), || {});
        let mut tree = new_tree(&root);
        let mut b = Builder::new();
        loop {
            let done = job.done();
            while let Some(m) = job.try_recv() {
                b.apply(&mut tree, m);
            }
            if done {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        finish(&mut tree, job.started.elapsed());

        let full = scan_blocking(&root, &Progress::default()).unwrap();
        let (a, f) = (tree.node(0), full.node(0));
        assert_eq!((a.size, a.files, a.dirs), (f.size, f.files, f.dirs));
        assert_eq!(tree.nodes.len(), full.nodes.len());
        // Sorted after finish, and folder colours agree.
        for i in 0..tree.nodes.len() as u32 {
            let kids: Vec<u64> = tree.children(i).map(|c| tree.node(c).size).collect();
            assert!(kids.windows(2).all(|w| w[0] >= w[1]), "children of {} not sorted", tree.path(i));
            if tree.node(i).is_dir {
                let other = full.find(&tree.path(i)).unwrap();
                assert_eq!(tree.node(i).cat, full.node(other).cat, "{}", tree.path(i));
            }
        }
        let _ = fs::remove_dir_all(&d);
    }
}
