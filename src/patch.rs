//! Live edits to a scanned tree: bring one folder in line with what's on disk, graft a newly
//! scanned subfolder in, and compact the arena once enough slots have gone unused.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::scan::{Listing, NONE, Node, Tree};
use crate::util::Cat;

enum Kid {
    Keep(u32),
    New(Node),
}

impl Tree {
    /// Make folder `d`'s children match `l`. Files that changed are re-measured, deleted items
    /// leave, new files arrive. New subfolders arrive empty: their paths are returned so the
    /// caller can scan them and `graft` the result.
    pub fn apply_listing(&mut self, d: u32, l: Listing) -> Vec<String> {
        let existing: HashMap<String, u32> = self.children(d).map(|c| (self.node(c).name.to_string(), c)).collect();
        let base = self.path(d);
        let mut kids = Vec::with_capacity(l.files.len() + l.dirs.len());
        let mut changed = false;
        let mut new_dirs = Vec::new();
        let mut kept = 0usize;
        for (name, size, mtime) in l.files {
            match existing.get(&*name) {
                Some(&c) if !self.node(c).is_dir => {
                    let n = &mut self.nodes[c as usize];
                    // Re-measure when the file changed. A size of 0 with the same mtime is a hard
                    // link counted once elsewhere: keep it at 0.
                    if n.mtime != mtime || (n.size != size && n.size != 0) {
                        changed |= n.size != size;
                        n.size = size;
                        n.mtime = mtime;
                    }
                    kids.push(Kid::Keep(c));
                    kept += 1;
                }
                _ => {
                    changed = true;
                    let cat = Cat::of(&name);
                    kids.push(Kid::New(Node {
                        name,
                        parent: d,
                        first_child: NONE,
                        n_children: 0,
                        cap: 0,
                        size,
                        files: 1,
                        dirs: 0,
                        mtime,
                        is_dir: false,
                        unreadable: false,
                        cat,
                    }));
                }
            }
        }
        for (name, mtime) in l.dirs {
            match existing.get(&*name) {
                Some(&c) if self.node(c).is_dir => {
                    kids.push(Kid::Keep(c));
                    kept += 1;
                }
                _ => {
                    changed = true;
                    new_dirs.push(if base.ends_with('/') { format!("{base}{name}") } else { format!("{base}/{name}") });
                    let cat = if name.ends_with(".app") { Cat::Apps } else { Cat::Other };
                    kids.push(Kid::New(Node {
                        name,
                        parent: d,
                        first_child: NONE,
                        n_children: 0,
                        cap: 0,
                        size: 0,
                        files: 0,
                        dirs: 0,
                        mtime,
                        is_dir: true,
                        unreadable: false,
                        cat,
                    }));
                }
            }
        }
        if changed || kept != existing.len() || self.node(d).unreadable {
            self.set_children(d, kids);
        }
        new_dirs
    }

    fn set_children(&mut self, d: u32, kids: Vec<Kid>) {
        let (old_first, old_n, old_cap) = {
            let n = self.node(d);
            (n.first_child, n.n_children, n.cap)
        };
        let kept: HashSet<u32> = kids.iter().filter_map(|k| if let Kid::Keep(i) = k { Some(*i) } else { None }).collect();
        for c in old_first..old_first.wrapping_add(old_n) {
            if !kept.contains(&c) {
                let n = self.node(c);
                // The child and everything under it is now unreachable.
                self.garbage += 1 + if n.is_dir { (n.files + n.dirs as u64) as usize } else { 0 };
            }
        }
        let mut new_nodes: Vec<Node> = kids
            .into_iter()
            .map(|k| match k {
                Kid::Keep(i) => std::mem::replace(&mut self.nodes[i as usize], Node::dead()),
                Kid::New(n) => n,
            })
            .collect();
        for c in old_first..old_first.wrapping_add(old_n) {
            self.nodes[c as usize] = Node::dead();
        }
        new_nodes.sort_unstable_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));

        let len = new_nodes.len() as u32;
        let (start, cap) = if len <= old_cap && (old_first != NONE || len == 0) {
            (old_first, old_cap)
        } else {
            // Move the children to the end with room to grow.
            self.garbage += old_cap as usize;
            let cap = len + len / 4 + 4;
            let start = self.nodes.len() as u32;
            self.nodes.resize_with((start + cap) as usize, Node::dead);
            (start, cap)
        };
        let (mut size, mut files, mut dirs) = (0u64, 0u64, 0u32);
        let mut by_cat = [0u64; Cat::ALL.len()];
        for (k, mut n) in new_nodes.into_iter().enumerate() {
            let slot = start + k as u32;
            n.parent = d;
            for c in n.first_child..n.first_child.wrapping_add(n.n_children) {
                self.nodes[c as usize].parent = slot;
            }
            size += n.size;
            files += n.files;
            if n.is_dir {
                dirs += 1 + n.dirs;
            }
            by_cat[n.cat as usize] += n.size;
            self.nodes[slot as usize] = n;
        }
        let node = &mut self.nodes[d as usize];
        let (dsize, dfiles, ddirs) =
            (size as i64 - node.size as i64, files as i64 - node.files as i64, dirs as i64 - node.dirs as i64);
        node.first_child = start;
        node.n_children = len;
        node.cap = cap;
        node.size = size;
        node.files = files;
        node.dirs = dirs;
        node.unreadable = false;
        if !node.name.ends_with(".app")
            && let Some((k, &v)) = by_cat.iter().enumerate().max_by_key(|(_, v)| **v)
            && v > 0
        {
            node.cat = Cat::ALL[k];
        }
        self.add_up(d, dsize, dfiles, ddirs);
    }

    /// Replace folder `g`'s contents with a freshly scanned subtree (`sub[0]` is its root).
    pub fn graft(&mut self, g: u32, sub: Vec<Node>) {
        let (old_first, old_n, old_cap) = {
            let n = self.node(g);
            (n.first_child, n.n_children, n.cap)
        };
        for c in old_first..old_first.wrapping_add(old_n) {
            let n = self.node(c);
            self.garbage += 1 + if n.is_dir { (n.files + n.dirs as u64) as usize } else { 0 };
            self.nodes[c as usize] = Node::dead();
        }
        self.garbage += old_cap.saturating_sub(old_n) as usize;

        let base = self.nodes.len() as u32;
        let map = |x: u32| if x == 0 { g } else { base + x - 1 };
        let mut it = sub.into_iter();
        let Some(root) = it.next() else { return };
        for mut n in it {
            n.parent = map(n.parent);
            n.first_child = if n.n_children > 0 { map(n.first_child) } else { NONE };
            self.nodes.push(n);
        }
        let node = &mut self.nodes[g as usize];
        let (dsize, dfiles, ddirs) = (
            root.size as i64 - node.size as i64,
            root.files as i64 - node.files as i64,
            root.dirs as i64 - node.dirs as i64,
        );
        node.first_child = if root.n_children > 0 { map(root.first_child) } else { NONE };
        node.n_children = root.n_children;
        node.cap = root.n_children;
        node.size = root.size;
        node.files = root.files;
        node.dirs = root.dirs;
        node.unreadable = root.unreadable;
        if !node.name.ends_with(".app") {
            node.cat = root.cat;
        }
        self.add_up(g, dsize, dfiles, ddirs);
    }

    /// Add a change in totals to every folder above `from`.
    fn add_up(&mut self, from: u32, dsize: i64, dfiles: i64, ddirs: i64) {
        let mut cur = self.node(from).parent;
        while cur != NONE {
            let n = &mut self.nodes[cur as usize];
            n.size = (n.size as i64 + dsize).max(0) as u64;
            n.files = (n.files as i64 + dfiles).max(0) as u64;
            n.dirs = (n.dirs as i64 + ddirs).max(0) as u32;
            cur = n.parent;
        }
    }

    /// Rebuild the arena without unused slots, each folder's children sorted largest first.
    /// Node indices change: callers re-find by path.
    pub fn compact(&mut self) {
        let mut old = std::mem::take(&mut self.nodes);
        let mut out: Vec<Node> = Vec::with_capacity(old.len().saturating_sub(self.garbage));
        let mut root = std::mem::replace(&mut old[0], Node::dead());
        root.parent = NONE;
        out.push(root);
        let mut queue = VecDeque::from([0u32]);
        while let Some(ni) = queue.pop_front() {
            let (f, n) = (out[ni as usize].first_child, out[ni as usize].n_children);
            let first = out.len() as u32;
            let mut kids: Vec<Node> =
                (f..f.wrapping_add(n)).map(|c| std::mem::replace(&mut old[c as usize], Node::dead())).collect();
            kids.sort_unstable_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
            for mut node in kids {
                node.parent = ni;
                if node.n_children == 0 {
                    node.first_child = NONE;
                    node.cap = 0;
                } else {
                    queue.push_back(out.len() as u32);
                }
                out.push(node);
            }
            let p = &mut out[ni as usize];
            p.first_child = if n > 0 { first } else { NONE };
            p.cap = n;
        }
        self.nodes = out;
        self.garbage = 0;
        std::thread::spawn(move || drop(old));
    }

    /// The deepest node on the way to `path` that is in the tree (at least the root).
    pub fn nearest(&self, path: &str) -> u32 {
        let mut p = path.trim_end_matches('/');
        loop {
            if let Some(i) = self.find(p) {
                return i;
            }
            match p.rfind('/') {
                Some(k) if k > 0 && p.len() > self.root_path.len() => p = &p[..k],
                _ => return 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::scan::{Progress, list_dir, scan_blocking};

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "spacious-patch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Totals must equal a fresh scan after any mix of edits.
    fn check(t: &crate::scan::Tree, root: &str) {
        let fresh = scan_blocking(root, &Progress::default()).unwrap();
        assert_eq!(t.node(0).size, fresh.node(0).size, "size");
        assert_eq!(t.node(0).files, fresh.node(0).files, "files");
        assert_eq!(t.node(0).dirs, fresh.node(0).dirs, "dirs");
        for i in 0..t.node(0).n_children {
            let c = t.node(0).first_child + i;
            assert_eq!(t.node(t.node(c).parent).name, t.node(0).name);
        }
    }

    #[test]
    fn delete_add_change_and_graft() {
        let d = tmp();
        let root = d.to_str().unwrap().to_owned();
        fs::create_dir_all(d.join("a/b")).unwrap();
        fs::write(d.join("a/one.bin"), vec![1u8; 100_000]).unwrap();
        fs::write(d.join("a/b/two.bin"), vec![1u8; 300_000]).unwrap();
        fs::write(d.join("top.txt"), vec![1u8; 5_000]).unwrap();
        let mut t = scan_blocking(&root, &Progress::default()).unwrap();

        // Delete a file deep down.
        fs::remove_file(d.join("a/b/two.bin")).unwrap();
        let b = t.find(&format!("{root}/a/b")).unwrap();
        assert!(t.apply_listing(b, list_dir(&d.join("a/b"), &[]).unwrap()).is_empty());
        check(&t, &root);

        // Grow a file and add another.
        fs::write(d.join("a/one.bin"), vec![1u8; 400_000]).unwrap();
        fs::write(d.join("a/new.png"), vec![1u8; 70_000]).unwrap();
        let a = t.find(&format!("{root}/a")).unwrap();
        assert!(t.apply_listing(a, list_dir(&d.join("a"), &[]).unwrap()).is_empty());
        check(&t, &root);

        // A new folder arrives empty, then its scan is grafted in.
        fs::create_dir_all(d.join("c/d")).unwrap();
        fs::write(d.join("c/d/x.mov"), vec![1u8; 250_000]).unwrap();
        let new = t.apply_listing(0, list_dir(&d, &[]).unwrap());
        assert_eq!(new, vec![format!("{root}/c")]);
        let sub = scan_blocking(&new[0], &Progress::default()).unwrap();
        let c = t.find(&new[0]).unwrap();
        t.graft(c, sub.nodes);
        check(&t, &root);
        assert!(t.find(&format!("{root}/c/d/x.mov")).is_some());

        // Delete a whole folder, then compact.
        fs::remove_dir_all(d.join("a")).unwrap();
        t.apply_listing(0, list_dir(&d, &[]).unwrap());
        check(&t, &root);
        assert!(t.find(&format!("{root}/a")).is_none());
        assert!(t.garbage > 0);
        t.compact();
        assert_eq!(t.garbage, 0);
        check(&t, &root);
        assert!(t.find(&format!("{root}/c/d/x.mov")).is_some());
        assert_eq!(t.nearest(&format!("{root}/c/d/nothing/here")), t.find(&format!("{root}/c/d")).unwrap());
        fs::remove_dir_all(&d).unwrap();
    }
}
