//! Squarified treemap layout, SpaceSniffer style. The current folder's items are big
//! rectangles; a folder shows its contents inside it only once it has been opened (clicked).
//! Items too small to see are merged into one cell per folder, so the number of cells is
//! bounded by the screen area, not by the number of files.

use std::collections::HashSet;

use egui::{Rect, pos2, vec2};

use crate::scan::Tree;

pub const HEADER_H: f32 = 18.0;
const PAD: f32 = 3.0;
/// Smallest cell drawn, in square points.
const MIN_AREA: f32 = 100.0;
/// A folder needs this much room before its contents are drawn inside it.
const MIN_DIR_W: f32 = 40.0;
const MIN_DIR_H: f32 = HEADER_H + 14.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Node(u32),
    /// Items of `parent` too small to draw, merged.
    Rest { parent: u32, count: u32, size: u64 },
}

#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub item: Item,
    pub rect: Rect,
    /// The folder is open: drawn with its contents and a header.
    pub open: bool,
}

/// Lay out `root`'s items in `rect`. Folders in `expanded` show their contents.
pub fn layout(tree: &Tree, root: u32, rect: Rect, expanded: &HashSet<u32>, out: &mut Vec<Cell>) {
    out.clear();
    layout_children(tree, root, rect, expanded, out);
}

/// Room inside an open folder's cell for its contents.
pub fn inner_rect(r: Rect) -> Rect {
    Rect::from_min_max(pos2(r.min.x + PAD, r.min.y + HEADER_H), pos2(r.max.x - PAD, r.max.y - PAD))
}

/// A folder this big can be opened in place.
pub fn can_open(r: Rect) -> bool {
    r.width() >= MIN_DIR_W && r.height() >= MIN_DIR_H
}

fn layout_children(tree: &Tree, dir: u32, rect: Rect, expanded: &HashSet<u32>, out: &mut Vec<Cell>) {
    let total = tree.node(dir).size;
    if total == 0 || rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    let scale = rect.area() as f64 / total as f64;
    let mut items: Vec<Item> = Vec::new();
    let mut values: Vec<f64> = Vec::new();
    let mut shown: u64 = 0;
    let mut rest_count = 0u32;
    for c in tree.children(dir) {
        let s = tree.node(c).size;
        if s == 0 {
            break; // sorted: the rest are empty too
        }
        if (s as f64 * scale) < MIN_AREA as f64 {
            rest_count = tree.children(dir).end - c;
            break;
        }
        items.push(Item::Node(c));
        values.push(s as f64);
        shown += s;
    }
    let rest = total - shown;
    if rest_count > 0 && (rest as f64 * scale) >= MIN_AREA as f64 {
        // Keep the list sorted so squarify stays well behaved.
        let pos = values.partition_point(|&v| v >= rest as f64);
        items.insert(pos, Item::Rest { parent: dir, count: rest_count, size: rest });
        values.insert(pos, rest as f64);
        shown += rest;
    }
    if items.is_empty() {
        return;
    }
    // Small leftovers are dropped from the layout; scale what's drawn to fill the rect.
    let k = rect.area() as f64 / shown as f64;
    for v in &mut values {
        *v *= k;
    }
    let mut rects = Vec::with_capacity(values.len());
    squarify(&values, rect, &mut rects);

    for (item, r) in items.into_iter().zip(rects) {
        let mut open = false;
        if let Item::Node(i) = item {
            let n = tree.node(i);
            open = n.is_dir && n.n_children > 0 && expanded.contains(&i) && can_open(r);
            out.push(Cell { item, rect: r, open });
            if open {
                layout_children(tree, i, inner_rect(r), expanded, out);
            }
        } else {
            out.push(Cell { item, rect: r, open });
        }
    }
}

/// Squarified treemap (Bruls, Huizing, van Wijk). `values` must sum to `rect`'s area and be
/// sorted largest first.
pub fn squarify(values: &[f64], rect: Rect, out: &mut Vec<Rect>) {
    let mut r = rect;
    let n = values.len();
    let mut i = 0;
    while i < n {
        let w = r.width().min(r.height()) as f64;
        if w <= 0.0 {
            for _ in i..n {
                out.push(Rect::from_min_size(r.min, vec2(0.0, 0.0)));
            }
            return;
        }
        let w2 = w * w;
        let mut j = i;
        let mut sum = 0.0;
        let mut min_v = f64::MAX;
        let mut max_v: f64 = 0.0;
        let mut best = f64::MAX;
        while j < n {
            let v = values[j];
            let s = sum + v;
            let mn = min_v.min(v);
            let mx = max_v.max(v);
            let s2 = s * s;
            let worst = (w2 * mx / s2).max(s2 / (w2 * mn));
            if j > i && worst > best {
                break;
            }
            best = worst;
            sum = s;
            min_v = mn;
            max_v = mx;
            j += 1;
        }
        let last = j == n;
        if r.width() >= r.height() {
            // A column on the left.
            let cw = if last { r.width() } else { ((sum / r.height() as f64) as f32).min(r.width()) };
            let mut y = r.min.y;
            for (k, v) in values[i..j].iter().enumerate() {
                let h = if i + k + 1 == j { r.max.y - y } else { (v / sum) as f32 * r.height() };
                out.push(Rect::from_min_size(pos2(r.min.x, y), vec2(cw, h)));
                y += h;
            }
            r.min.x += cw;
        } else {
            // A row along the top.
            let rh = if last { r.height() } else { ((sum / r.width() as f64) as f32).min(r.height()) };
            let mut x = r.min.x;
            for (k, v) in values[i..j].iter().enumerate() {
                let w = if i + k + 1 == j { r.max.x - x } else { (v / sum) as f32 * r.width() };
                out.push(Rect::from_min_size(pos2(x, r.min.y), vec2(w, rh)));
                x += w;
            }
            r.min.y += rh;
        }
        i = j;
    }
}

/// Deepest cell under `pos`.
pub fn hit(cells: &[Cell], pos: egui::Pos2) -> Option<usize> {
    cells.iter().rposition(|c| c.rect.contains(pos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squarify_fills_rect() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0));
        let raw = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let sum: f64 = raw.iter().sum();
        let vals: Vec<f64> = raw.iter().map(|v| v / sum * 240_000.0).collect();
        let mut out = Vec::new();
        squarify(&vals, rect, &mut out);
        assert_eq!(out.len(), raw.len());
        let area: f32 = out.iter().map(|r| r.area()).sum();
        assert!((area - 240_000.0).abs() < 1.0, "area {area}");
        for (r, v) in out.iter().zip(&vals) {
            assert!((r.area() as f64 - v).abs() / v < 0.01, "{r:?} vs {v}");
            assert!(rect.expand(0.01).contains_rect(*r));
        }
    }
}
