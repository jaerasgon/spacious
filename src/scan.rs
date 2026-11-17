//! Parallel disk scanner and the compact tree it produces.
//!
//! The whole tree lives in one `Vec<Node>`. Children of a folder are contiguous and sorted by
//! size, largest first, so the views never sort or copy. Nothing is dropped: every file counts
//! toward its folder's total, however many there are.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use rayon::prelude::*;

use crate::util::Cat;

pub const NONE: u32 = u32::MAX;

pub struct Node {
    pub name: Box<str>,
    pub parent: u32,
    pub first_child: u32,
    pub n_children: u32,
    /// Slots reserved for children from `first_child`. Live updates fill spare slots in place
    /// before they have to move a folder's children to the end of the arena.
    pub cap: u32,
    /// Bytes on disk (allocated blocks), so sparse files such as Docker.raw count what they use.
    pub size: u64,
    /// Files in this subtree (1 for a file).
    pub files: u64,
    /// Folders in this subtree, not counting itself.
    pub dirs: u32,
    pub mtime: i64,
    pub is_dir: bool,
    /// The folder could not be read (permission denied).
    pub unreadable: bool,
    pub cat: Cat,
}

pub struct Tree {
    pub nodes: Vec<Node>,
    pub root_path: String,
    pub unreadable: u64,
    pub elapsed: Duration,
    /// Slots no longer reachable after live updates; `compact` reclaims them.
    pub garbage: usize,
}

impl Node {
    /// An unused slot.
    pub fn dead() -> Node {
        Node {
            name: Box::default(),
            parent: NONE,
            first_child: NONE,
            n_children: 0,
            cap: 0,
            size: 0,
            files: 0,
            dirs: 0,
            mtime: 0,
            is_dir: false,
            unreadable: false,
            cat: Cat::Other,
        }
    }
}

/// One folder's entries, read without descending.
pub struct Listing {
    pub files: Vec<(Box<str>, u64, i64)>,
    pub dirs: Vec<(Box<str>, i64)>,
}

/// Read one folder: files with their size on disk, and the names of its subfolders.
pub fn list_dir(path: &Path, skip: &[PathBuf]) -> Option<Listing> {
    let rd = fs::read_dir(path).ok()?;
    let mut l = Listing { files: Vec::new(), dirs: Vec::new() };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let name: Box<str> = entry.file_name().to_string_lossy().into();
        if ft.is_dir() {
            if skip.iter().any(|s| s == &entry.path()) {
                continue;
            }
            l.dirs.push((name, entry.metadata().map(|m| m.mtime()).unwrap_or(0)));
        } else if ft.is_file()
            && let Ok(md) = entry.metadata()
        {
            l.files.push((name, md.blocks() * 512, md.mtime()));
        }
    }
    Some(l)
}

impl Tree {
    #[inline]
    pub fn node(&self, i: u32) -> &Node {
        &self.nodes[i as usize]
    }

    #[inline]
    pub fn children(&self, i: u32) -> std::ops::Range<u32> {
        let n = self.node(i);
        n.first_child..n.first_child + n.n_children
    }

    pub fn path(&self, i: u32) -> String {
        let mut parts = Vec::new();
        let mut cur = i;
        while cur != 0 && cur != NONE {
            parts.push(&*self.node(cur).name);
            cur = self.node(cur).parent;
        }
        let mut s = self.root_path.clone();
        for p in parts.iter().rev() {
            if !s.ends_with('/') {
                s.push('/');
            }
            s.push_str(p);
        }
        s
    }

    /// Chain from the tree root to `i`, inclusive.
    pub fn ancestors(&self, i: u32) -> Vec<u32> {
        let mut v = vec![i];
        let mut cur = i;
        while cur != 0 {
            cur = self.node(cur).parent;
            v.push(cur);
        }
        v.reverse();
        v
    }

    /// Find a node by absolute path, if it is inside this tree.
    pub fn find(&self, path: &str) -> Option<u32> {
        let rel = path.strip_prefix(self.root_path.as_str())?;
        let mut cur = 0u32;
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            cur = self.children(cur).find(|&c| &*self.node(c).name == part)?;
        }
        Some(cur)
    }

    /// Largest files in the subtree of `i`.
    pub fn top_files(&self, i: u32, k: usize) -> Vec<u32> {
        let mut top: Vec<u32> = Vec::with_capacity(k + 1);
        let mut stack = vec![i];
        while let Some(n) = stack.pop() {
            let node = self.node(n);
            if !node.is_dir {
                if top.len() < k || node.size > self.node(*top.last().unwrap()).size {
                    let pos = top.partition_point(|&t| self.node(t).size >= node.size);
                    top.insert(pos, n);
                    top.truncate(k);
                }
                continue;
            }
            for c in self.children(n) {
                // Children are sorted, so a folder smaller than the current k-th file can't help.
                let cn = self.node(c);
                if top.len() == k && cn.size <= self.node(top[k - 1]).size {
                    break;
                }
                stack.push(c);
            }
        }
        top
    }
}

/// Live counters, read by the UI while a scan runs.
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub unreadable: AtomicU64,
    pub current: Mutex<String>,
    pub cancel: AtomicBool,
}

pub struct ScanJob {
    pub root: String,
    pub progress: Arc<Progress>,
    pub started: Instant,
    rx: mpsc::Receiver<Option<Tree>>,
}

impl ScanJob {
    /// `Some(Some(tree))` when done, `Some(None)` when cancelled or failed, `None` while running.
    pub fn poll(&self) -> Option<Option<Tree>> {
        match self.rx.try_recv() {
            Ok(t) => Some(t),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(None),
        }
    }

    pub fn cancel(&self) {
        self.progress.cancel.store(true, Ordering::Relaxed);
    }
}

/// Start scanning `root` on background threads. `wake` is called when the scan finishes.
pub fn start(root: String, wake: impl Fn() + Send + 'static) -> ScanJob {
    let progress = Arc::new(Progress::default());
    let (tx, rx) = mpsc::channel();
    let p = progress.clone();
    let r = root.clone();
    std::thread::Builder::new()
        .name("spacious-scan".into())
        .spawn(move || {
            let tree = scan_blocking(&r, &p);
            let _ = tx.send(tree);
            wake();
        })
        .expect("spawn scan thread");
    ScanJob { root, progress, started: Instant::now(), rx }
}

struct Ctx<'a> {
    p: &'a Progress,
    /// Inodes of files with more than one hard link, so each is counted once.
    links: Vec<Mutex<HashSet<(u64, u64)>>>,
    skip: Vec<PathBuf>,
}

impl Ctx<'_> {
    fn first_link(&self, dev: u64, ino: u64) -> bool {
        let shard = (ino as usize) % self.links.len();
        self.links[shard].lock().unwrap().insert((dev, ino))
    }
    fn cancelled(&self) -> bool {
        self.p.cancel.load(Ordering::Relaxed)
    }
}

/// Intermediate tree built by the parallel walk, flattened afterwards.
struct Raw {
    name: Box<str>,
    size: u64,
    files: u64,
    dirs: u32,
    mtime: i64,
    is_dir: bool,
    unreadable: bool,
    children: Vec<Raw>,
}

impl Raw {
    fn dir(name: Box<str>, mtime: i64, children: Vec<Raw>, unreadable: bool) -> Raw {
        let mut size = 0;
        let mut files = 0;
        let mut dirs = 0;
        for c in &children {
            size += c.size;
            files += c.files;
            if c.is_dir {
                dirs += 1 + c.dirs;
            }
        }
        Raw { name, size, files, dirs, mtime, is_dir: true, unreadable, children }
    }
}

/// Folders that are other volumes or mirrors of the same data when scanning from `/`.
pub fn skip_list(root: &Path) -> Vec<PathBuf> {
    ["/dev", "/Volumes", "/System/Volumes", "/net", "/home", "/proc", "/sys"]
        .iter()
        .map(PathBuf::from)
        .filter(|p| !root.starts_with(p))
        .collect()
}

pub fn scan_blocking(root: &str, p: &Progress) -> Option<Tree> {
    let started = Instant::now();
    let root_path = PathBuf::from(root);
    let md = fs::symlink_metadata(&root_path).ok()?;
    if !md.is_dir() {
        return None;
    }
    let ctx = Ctx { p, links: (0..64).map(|_| Mutex::new(HashSet::new())).collect(), skip: skip_list(&root_path) };

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(std::thread::available_parallelism().map(|n| n.get() * 2).unwrap_or(8).clamp(4, 32))
        .stack_size(8 * 1024 * 1024)
        .thread_name(|i| format!("spacious-walk-{i}"))
        .build()
        .ok()?;
    let (children, unreadable) = pool.install(|| scan_dir(&root_path, &ctx));
    if ctx.cancelled() {
        return None;
    }
    let raw = Raw::dir(root.into(), md.mtime(), children, unreadable);
    drop(ctx);
    let mut nodes = flatten(raw);
    dominant_cats(&mut nodes);
    Some(Tree {
        nodes,
        root_path: root.to_owned(),
        unreadable: p.unreadable.load(Ordering::Relaxed),
        elapsed: started.elapsed(),
        garbage: 0,
    })
}

fn scan_dir(path: &Path, ctx: &Ctx) -> (Vec<Raw>, bool) {
    if ctx.cancelled() {
        return (Vec::new(), false);
    }
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) => {
            ctx.p.unreadable.fetch_add(1, Ordering::Relaxed);
            return (Vec::new(), true);
        }
    };
    if let Ok(mut cur) = ctx.p.current.try_lock() {
        cur.clear();
        cur.push_str(&path.to_string_lossy());
    }

    let mut out = Vec::new();
    let mut subdirs: Vec<(Box<str>, PathBuf, i64)> = Vec::new();
    let mut nfiles = 0u64;
    let mut nbytes = 0u64;
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
            subdirs.push((name, p, mtime));
        } else if ft.is_file() {
            let Ok(md) = entry.metadata() else { continue };
            let mut size = md.blocks() * 512;
            if md.nlink() > 1 && !ctx.first_link(md.dev(), md.ino()) {
                size = 0;
            }
            nfiles += 1;
            nbytes += size;
            out.push(Raw {
                name,
                size,
                files: 1,
                dirs: 0,
                mtime: md.mtime(),
                is_dir: false,
                unreadable: false,
                children: Vec::new(),
            });
        }
    }
    ctx.p.files.fetch_add(nfiles, Ordering::Relaxed);
    ctx.p.bytes.fetch_add(nbytes, Ordering::Relaxed);
    ctx.p.dirs.fetch_add(1, Ordering::Relaxed);

    let sub: Vec<Raw> = subdirs
        .into_par_iter()
        .map(|(name, p, mtime)| {
            let (children, unreadable) = scan_dir(&p, ctx);
            Raw::dir(name, mtime, children, unreadable)
        })
        .collect();
    out.extend(sub);
    (out, false)
}

/// Breadth-first copy into the arena, so each folder's children sit next to each other.
fn flatten(root: Raw) -> Vec<Node> {
    let total = 1 + root.files as usize + root.dirs as usize;
    let mut nodes: Vec<Node> = Vec::with_capacity(total);
    let mut root = root;
    let kids = std::mem::take(&mut root.children);
    nodes.push(make_node(&mut root, NONE));
    let mut queue: VecDeque<(Vec<Raw>, u32)> = VecDeque::new();
    queue.push_back((kids, 0));
    while let Some((mut kids, idx)) = queue.pop_front() {
        kids.sort_unstable_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
        let first = nodes.len() as u32;
        nodes[idx as usize].first_child = first;
        nodes[idx as usize].n_children = kids.len() as u32;
        nodes[idx as usize].cap = kids.len() as u32;
        for mut k in kids {
            let grand = std::mem::take(&mut k.children);
            let i = nodes.len() as u32;
            nodes.push(make_node(&mut k, idx));
            if !grand.is_empty() {
                queue.push_back((grand, i));
            }
        }
    }
    nodes
}

/// Give each folder the category of the files that take most of its space, so folders can
/// be coloured by what's in them. Children always sit after their parent in the arena, so one
/// backward pass sees every folder's contents before the folder itself.
fn dominant_cats(nodes: &mut [Node]) {
    let mut pending: std::collections::HashMap<u32, [u64; Cat::ALL.len()]> = std::collections::HashMap::new();
    for i in (0..nodes.len()).rev() {
        if !nodes[i].is_dir {
            continue;
        }
        let mut by_cat = [0u64; Cat::ALL.len()];
        let n = &nodes[i];
        if n.n_children > 0 {
            for c in n.first_child..n.first_child + n.n_children {
                let child = &nodes[c as usize];
                if child.is_dir {
                    if let Some(sub) = pending.remove(&c) {
                        for (a, b) in by_cat.iter_mut().zip(sub) {
                            *a += b;
                        }
                    }
                } else {
                    by_cat[child.cat as usize] += child.size;
                }
            }
        }
        let n = &mut nodes[i];
        if n.name.ends_with(".app") {
            n.cat = Cat::Apps;
        } else if let Some((k, &v)) = by_cat.iter().enumerate().max_by_key(|(_, v)| **v)
            && v > 0
        {
            n.cat = Cat::ALL[k];
        }
        if n.parent != NONE {
            pending.insert(i as u32, by_cat);
        }
    }
}

fn make_node(r: &mut Raw, parent: u32) -> Node {
    let name = std::mem::take(&mut r.name);
    let cat = if r.is_dir {
        if name.ends_with(".app") { Cat::Apps } else { Cat::Other }
    } else {
        Cat::of(&name)
    };
    Node {
        name,
        parent,
        first_child: NONE,
        n_children: 0,
        cap: 0,
        size: r.size,
        files: r.files,
        dirs: r.dirs,
        mtime: r.mtime,
        is_dir: r.is_dir,
        unreadable: r.unreadable,
        cat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("spacious-test-{}-{}", std::process::id(), rand_u64()));
        fs::create_dir_all(&d).unwrap();
        d
    }
    fn rand_u64() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
    }

    #[test]
    fn scans_counts_and_sorts() {
        let d = tmp();
        fs::create_dir_all(d.join("a/b")).unwrap();
        fs::write(d.join("small.txt"), vec![1u8; 10]).unwrap();
        fs::write(d.join("a/big.bin"), vec![1u8; 200_000]).unwrap();
        fs::write(d.join("a/b/mid.png"), vec![1u8; 50_000]).unwrap();
        fs::hard_link(d.join("a/big.bin"), d.join("a/b/link.bin")).unwrap();
        std::os::unix::fs::symlink(d.join("a"), d.join("loop")).unwrap();

        let p = Progress::default();
        let t = scan_blocking(d.to_str().unwrap(), &p).unwrap();
        let root = t.node(0);
        assert_eq!(root.files, 4);
        assert_eq!(root.dirs, 2);
        // Hard link counted once, symlink skipped.
        let a = t.children(0).find(|&c| &*t.node(c).name == "a").unwrap();
        assert_eq!(t.children(0).next(), Some(a), "largest child first");
        let big = t.find(&format!("{}/a/big.bin", d.display())).unwrap();
        assert_eq!(t.path(big), format!("{}/a/big.bin", d.display()));
        let sum: u64 = t.children(0).map(|c| t.node(c).size).sum();
        assert_eq!(sum, root.size);
        assert!(root.size < 2 * 200_000 + 100_000, "hard link double counted: {}", root.size);
        // Whichever of the two hard links was seen first carries the size.
        let link = t.find(&format!("{}/a/b/link.bin", d.display())).unwrap();
        assert!([big, link].contains(&t.top_files(0, 2)[0]));
        // Folder colour follows its biggest file type.
        assert_eq!(t.node(a).cat, Cat::Data, ".bin files are most of a");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn many_files() {
        let d = tmp();
        for i in 0..30 {
            let sub = d.join(format!("d{i}"));
            fs::create_dir_all(&sub).unwrap();
            for j in 0..1000 {
                fs::write(sub.join(format!("f{j}.txt")), b"x").unwrap();
            }
        }
        let p = Progress::default();
        let t = scan_blocking(d.to_str().unwrap(), &p).unwrap();
        assert_eq!(t.node(0).files, 30_000);
        assert_eq!(t.nodes.len(), 1 + 30 + 30_000);
        fs::remove_dir_all(&d).unwrap();
    }
}
