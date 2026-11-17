//! Formatting and file type categories.

use egui::Color32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Cat {
    Images,
    Documents,
    Media,
    Archives,
    Code,
    Fonts,
    Data,
    Design,
    Apps,
    Other,
}

impl Cat {
    pub const ALL: [Cat; 10] = [
        Cat::Images,
        Cat::Documents,
        Cat::Media,
        Cat::Archives,
        Cat::Code,
        Cat::Fonts,
        Cat::Data,
        Cat::Design,
        Cat::Apps,
        Cat::Other,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Cat::Images => "Images",
            Cat::Documents => "Documents",
            Cat::Media => "Media",
            Cat::Archives => "Archives",
            Cat::Code => "Code",
            Cat::Fonts => "Fonts",
            Cat::Data => "Data",
            Cat::Design => "Design",
            Cat::Apps => "Apps",
            Cat::Other => "Other",
        }
    }

    /// The same palette as the web version, so files keep their colours.
    pub fn color(self) -> Color32 {
        match self {
            Cat::Images => Color32::from_rgb(0x6b, 0xcb, 0x77),
            Cat::Documents => Color32::from_rgb(0xf5, 0xc8, 0x42),
            Cat::Media => Color32::from_rgb(0xa7, 0x8b, 0xfa),
            Cat::Archives => Color32::from_rgb(0xf5, 0x92, 0x4e),
            Cat::Code => Color32::from_rgb(0xe0, 0x5c, 0x5c),
            Cat::Fonts => Color32::from_rgb(0x60, 0xa5, 0xfa),
            Cat::Data => Color32::from_rgb(0x34, 0xd3, 0x99),
            Cat::Design => Color32::from_rgb(0xf4, 0x72, 0xb6),
            Cat::Apps => Color32::from_rgb(0xc9, 0xc9, 0xd6),
            Cat::Other => Color32::from_rgb(0x7a, 0xaa, 0xb3),
        }
    }

    pub fn of(name: &str) -> Cat {
        let Some(dot) = name.rfind('.') else { return Cat::Other };
        if dot == 0 {
            return Cat::Other;
        }
        let ext = &name[dot + 1..];
        if ext.len() > 10 {
            return Cat::Other;
        }
        let mut buf = [0u8; 10];
        for (i, b) in ext.bytes().enumerate() {
            buf[i] = b.to_ascii_lowercase();
        }
        let ext = std::str::from_utf8(&buf[..ext.len()]).unwrap_or("");
        match ext {
            "jpg" | "jpeg" | "png" | "gif" | "svg" | "webp" | "bmp" | "ico" | "tiff" | "tif" | "heic" | "avif"
            | "raw" | "cr2" | "nef" | "arw" | "jfif" | "pnm" | "exr" | "dng" | "hdr" => Cat::Images,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "rtf" | "odt" | "ods"
            | "odp" | "pages" | "numbers" | "key" | "keynote" | "epub" | "log" | "tex" => Cat::Documents,
            "mp4" | "mov" | "avi" | "mkv" | "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "wmv" | "flv"
            | "webm" | "m4v" | "opus" | "wma" | "aiff" | "aif" | "mid" | "midi" | "3gp" | "mxf" | "braw" | "r3d"
            | "prores" => Cat::Media,
            "zip" | "tar" | "gz" | "bz2" | "rar" | "7z" | "dmg" | "iso" | "xz" | "tgz" | "pkg" | "deb" | "rpm"
            | "cab" | "lz4" | "zst" | "sparseimage" | "sparsebundle" | "img" | "crate" => Cat::Archives,
            "js" | "ts" | "jsx" | "tsx" | "py" | "rb" | "go" | "rs" | "c" | "cpp" | "cc" | "h" | "hpp" | "cs"
            | "java" | "swift" | "kt" | "dart" | "php" | "html" | "css" | "scss" | "sass" | "less" | "vue"
            | "svelte" | "sh" | "bash" | "zsh" | "ps1" | "lua" | "r" | "m" | "pl" | "ex" | "exs" | "elm" | "clj"
            | "hs" | "gd" | "gdshader" | "glsl" | "wgsl" | "metal" | "mjs" | "cjs" => Cat::Code,
            "ttf" | "otf" | "woff" | "woff2" | "eot" | "fon" | "pfb" | "pfm" | "afm" | "ttc" => Cat::Fonts,
            "json" | "xml" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "env" | "sql" | "db" | "sqlite" | "csv"
            | "tsv" | "graphql" | "proto" | "parquet" | "npy" | "safetensors" | "gguf" | "ckpt" | "pt" | "bin"
            | "dat" | "plist" => Cat::Data,
            "psd" | "ai" | "indd" | "xd" | "fig" | "sketch" | "eps" | "afdesign" | "afphoto" | "afpub" | "af"
            | "cdr" | "blend" | "blend1" | "c4d" | "obj" | "fbx" | "stl" | "glb" | "gltf" | "usd" | "usdz"
            | "aep" | "prproj" | "drp" | "aura" | "pulse" => Cat::Design,
            "app" | "dylib" | "so" | "dll" | "exe" | "a" | "o" | "rlib" | "framework" | "wasm" | "jar" => Cat::Apps,
            _ => Cat::Other,
        }
    }
}

pub fn fmt_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

pub fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Local date and time, "2026-04-10 14:01".
pub fn fmt_time(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    // SAFETY: localtime_r only writes into the tm we own.
    unsafe {
        let t: libc::time_t = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return String::new();
        }
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min
        )
    }
}

pub fn fmt_duration(d: std::time::Duration) -> String {
    let s = d.as_secs_f64();
    if s < 60.0 { format!("{s:.1} s") } else { format!("{}m {:02}s", (s / 60.0) as u64, (s % 60.0) as u64) }
}

pub fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

/// Replace the home prefix with ~ for display.
pub fn tilde(path: &str) -> String {
    let home = home_dir();
    if home.len() > 1 && path.starts_with(&home) { format!("~{}", &path[home.len()..]) } else { path.to_owned() }
}

/// Expand a leading ~ and trim trailing slashes.
pub fn expand(path: &str) -> String {
    let p = path.trim();
    let p = if p == "~" {
        home_dir()
    } else if let Some(rest) = p.strip_prefix("~/") {
        format!("{}/{}", home_dir(), rest)
    } else {
        p.to_owned()
    };
    let t = p.trim_end_matches('/');
    if t.is_empty() { "/".into() } else { t.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(fmt_size(0), "0 B");
        assert_eq!(fmt_size(1023), "1023 B");
        assert_eq!(fmt_size(1536), "1.5 KB");
        assert_eq!(fmt_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn counts() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1000), "1,000");
        assert_eq!(fmt_count(1234567), "1,234,567");
    }

    #[test]
    fn cats() {
        assert_eq!(Cat::of("a.PNG"), Cat::Images);
        assert_eq!(Cat::of(".bashrc"), Cat::Other);
        assert_eq!(Cat::of("noext"), Cat::Other);
        assert_eq!(Cat::of("x.tar.gz"), Cat::Archives);
        assert_eq!(Cat::of("scene.blend"), Cat::Design);
    }
}
