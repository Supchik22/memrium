//! Archives: what is inside one, taking files out of it, and making one. zip, tar (plain, gzip, bzip2, xz, zstd), 7z, and
//! the single files a compressor leaves (`.gz`, `.bz2`, `.xz`, `.zst`).
//!
//! Taking files out never writes outside the folder it was asked to fill: an entry whose name goes up (`../x`) or starts at
//! the root is skipped, and so is one that would have to pass through a link; a link in the archive is made only when it
//! points inside that folder. The permission bits setuid and setgid are not kept.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    SevenZ,
    /// one file, compressed
    Gz,
    Bz2,
    Xz,
    Zst,
}

/// What an archive can be made as.
pub const CREATE: [Format; 7] = [Format::Zip, Format::TarGz, Format::TarXz, Format::TarZst, Format::TarBz2, Format::Tar, Format::SevenZ];

impl Format {
    /// From the file name.
    pub fn detect(path: &Path) -> Option<Format> {
        let n = path.file_name()?.to_string_lossy().to_lowercase();
        let ends = |s: &[&str]| s.iter().any(|e| n.ends_with(e));
        Some(if ends(&[".tar.gz", ".tgz"]) {
            Format::TarGz
        } else if ends(&[".tar.bz2", ".tbz2", ".tbz"]) {
            Format::TarBz2
        } else if ends(&[".tar.xz", ".txz"]) {
            Format::TarXz
        } else if ends(&[".tar.zst", ".tzst", ".tar.zstd"]) {
            Format::TarZst
        } else if ends(&[".tar"]) {
            Format::Tar
        } else if ends(&[".zip", ".jar", ".apk", ".cbz", ".whl", ".xpi"]) {
            Format::Zip
        } else if ends(&[".7z"]) {
            Format::SevenZ
        } else if ends(&[".gz"]) {
            Format::Gz
        } else if ends(&[".bz2"]) {
            Format::Bz2
        } else if ends(&[".xz"]) {
            Format::Xz
        } else if ends(&[".zst"]) {
            Format::Zst
        } else {
            return None;
        })
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::TarBz2 => "tar.bz2",
            Format::TarXz => "tar.xz",
            Format::TarZst => "tar.zst",
            Format::SevenZ => "7z",
            Format::Gz => "gz",
            Format::Bz2 => "bz2",
            Format::Xz => "xz",
            Format::Zst => "zst",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "ZIP",
            Format::Tar => "TAR",
            Format::TarGz => "TAR.GZ",
            Format::TarBz2 => "TAR.BZ2",
            Format::TarXz => "TAR.XZ",
            Format::TarZst => "TAR.ZST",
            Format::SevenZ => "7Z",
            Format::Gz => "GZ",
            Format::Bz2 => "BZ2",
            Format::Xz => "XZ",
            Format::Zst => "ZST",
        }
    }

    fn single(self) -> bool {
        matches!(self, Format::Gz | Format::Bz2 | Format::Xz | Format::Zst)
    }
}

/// Is this file one we can open as an archive?
pub fn is_archive(path: &Path) -> bool {
    !path.is_dir() && Format::detect(path).is_some()
}

/// One thing in an archive.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// the name as the archive has it, with `/` between the parts
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// seconds since 1970, when the archive says
    pub modified: Option<u64>,
}

/// How far a job has come; the callback of `extract` and `create` gets it often and returns false to stop.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub entries: u64,
    pub bytes: u64,
    pub current: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// entries left out (unsafe names, links pointing outside, kinds of file that cannot be made)
    pub skipped: u64,
}

type Cb<'a> = &'a mut dyn FnMut(&Progress) -> bool;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

const CANCELLED: &str = "Cancelled";

// ------------------------------------------------------------------ reading

fn tar_reader(path: &Path, format: Format) -> Result<Box<dyn Read>, String> {
    let file = BufReader::new(File::open(path).map_err(err)?);
    Ok(match format {
        Format::Tar => Box::new(file),
        Format::TarGz => Box::new(flate2::read::MultiGzDecoder::new(file)),
        Format::TarBz2 => Box::new(bzip2::read::MultiBzDecoder::new(file)),
        Format::TarXz => Box::new(xz2::read::XzDecoder::new_multi_decoder(file)),
        Format::TarZst => Box::new(zstd::stream::read::Decoder::with_buffer(file).map_err(err)?),
        _ => return Err("not a tar".into()),
    })
}

fn single_reader(path: &Path, format: Format) -> Result<Box<dyn Read>, String> {
    let file = BufReader::new(File::open(path).map_err(err)?);
    Ok(match format {
        Format::Gz => Box::new(flate2::read::MultiGzDecoder::new(file)),
        Format::Bz2 => Box::new(bzip2::read::MultiBzDecoder::new(file)),
        Format::Xz => Box::new(xz2::read::XzDecoder::new_multi_decoder(file)),
        Format::Zst => Box::new(zstd::stream::read::Decoder::with_buffer(file).map_err(err)?),
        _ => return Err("not a compressed file".into()),
    })
}

/// The name of the file inside a `.gz` and the like: the name without the ending.
fn single_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "file".to_string())
}

/// Seconds since 1970 of a calendar time (taken as UTC).
fn unix_time(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> u64 {
    // days from civil (Howard Hinnant's algorithm)
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    (days * 86400 + h * 3600 + mi * 60 + s).max(0) as u64
}

pub fn list(path: &Path) -> Result<Vec<Entry>, String> {
    let format = Format::detect(path).ok_or("Not an archive this program opens")?;
    let mut out = Vec::new();
    match format {
        Format::Zip => {
            let mut z = zip::ZipArchive::new(BufReader::new(File::open(path).map_err(err)?)).map_err(err)?;
            for i in 0..z.len() {
                let f = z.by_index_raw(i).map_err(err)?;
                let modified = f.last_modified().map(|t| unix_time(t.year() as i64, t.month() as i64, t.day() as i64, t.hour() as i64, t.minute() as i64, t.second() as i64));
                out.push(Entry { path: f.name().replace('\\', "/"), is_dir: f.is_dir(), size: f.size(), modified });
            }
        }
        Format::SevenZ => {
            let r = sevenz_rust2::SevenZReader::open(path, sevenz_rust2::Password::empty()).map_err(err)?;
            for e in r.archive().files.iter() {
                let modified = e.has_last_modified_date.then(|| std::time::SystemTime::from(e.last_modified_date).duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
                out.push(Entry { path: e.name.replace('\\', "/"), is_dir: e.is_directory, size: e.size, modified });
            }
        }
        f if f.single() => out.push(Entry { path: single_name(path), is_dir: false, size: 0, modified: None }),
        f => {
            let mut a = tar::Archive::new(tar_reader(path, f)?);
            for e in a.entries().map_err(err)? {
                let e = e.map_err(err)?;
                let name = e.path().map_err(err)?.to_string_lossy().to_string();
                out.push(Entry { path: name.trim_end_matches('/').to_string(), is_dir: e.header().entry_type().is_dir(), size: e.header().size().unwrap_or(0), modified: e.header().mtime().ok() });
            }
        }
    }
    // a folder that only exists because files are inside it is listed too
    let mut have: std::collections::HashSet<String> = out.iter().filter(|e| e.is_dir).map(|e| e.path.trim_end_matches('/').to_string()).collect();
    let mut extra = Vec::new();
    for e in &out {
        let mut p = e.path.trim_end_matches('/');
        while let Some((parent, _)) = p.rsplit_once('/') {
            if !parent.is_empty() && have.insert(parent.to_string()) {
                extra.push(Entry { path: parent.to_string(), is_dir: true, size: 0, modified: None });
            }
            p = parent;
        }
    }
    out.extend(extra);
    for e in &mut out {
        e.path = e.path.trim_end_matches('/').to_string();
    }
    Ok(out)
}

// ------------------------------------------------------------------ extracting

/// Where an entry goes below `dest`; None for a name that is empty, absolute or climbs out.
fn target(dest: &Path, name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    let mut out = dest.to_path_buf();
    let mut any = false;
    for c in Path::new(name.trim_start_matches("./")).components() {
        match c {
            Component::Normal(p) => {
                out.push(p);
                any = true;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    any.then_some(out)
}

/// Does `name` belong to the selection (a folder in it takes everything inside)?
fn wanted(only: &[String], name: &str) -> bool {
    let name = name.trim_end_matches('/');
    only.is_empty() || only.iter().any(|o| {
        let o = o.trim_end_matches('/');
        name == o || name.strip_prefix(o).is_some_and(|r| r.starts_with('/'))
    })
}

/// Make the folder a file goes in, refusing to pass through a link that is already there.
fn make_parent(dest: &Path, file: &Path) -> Result<(), String> {
    let parent = file.parent().ok_or("no folder")?;
    let rel = parent.strip_prefix(dest).map_err(|_| "outside the folder".to_string())?;
    let mut at = dest.to_path_buf();
    for c in rel.components() {
        at.push(c);
        match fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => return Err("passes through a link".into()),
            Ok(m) if !m.is_dir() => return Err("a file is in the way".into()),
            Ok(_) => {}
            Err(_) => fs::create_dir(&at).map_err(err)?,
        }
    }
    Ok(())
}

/// Is a link to `link_target`, made at `at`, still inside `dest`? (Worked out from the names, nothing on disk is read.)
fn link_stays_inside(dest: &Path, at: &Path, link_target: &Path) -> bool {
    if link_target.is_absolute() {
        return false;
    }
    let mut p = at.parent().unwrap_or(dest).to_path_buf();
    for c in link_target.components() {
        match c {
            Component::Normal(n) => p.push(n),
            Component::ParentDir => {
                if !p.pop() {
                    return false;
                }
            }
            Component::CurDir => {}
            _ => return false,
        }
    }
    p.starts_with(dest)
}

fn copy_out(reader: &mut dyn Read, to: &Path, mode: Option<u32>, prog: &mut Progress, cb: Cb<'_>) -> Result<u64, String> {
    if let Ok(m) = fs::symlink_metadata(to) {
        if m.is_dir() {
            return Err("a folder is in the way".into());
        }
        let _ = fs::remove_file(to);
    }
    let mut out = BufWriter::new(File::create(to).map_err(err)?);
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(err)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(err)?;
        total += n as u64;
        prog.bytes += n as u64;
        if !cb(prog) {
            drop(out);
            let _ = fs::remove_file(to);
            return Err(CANCELLED.into());
        }
    }
    out.flush().map_err(err)?;
    drop(out);
    if let Some(m) = mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(to, fs::Permissions::from_mode(m & 0o777));
    }
    Ok(total)
}

fn make_link(dest: &Path, at: &Path, link_target: &str) -> Result<bool, String> {
    if !link_stays_inside(dest, at, Path::new(link_target)) {
        return Ok(false);
    }
    if fs::symlink_metadata(at).is_ok() {
        let _ = fs::remove_file(at);
    }
    std::os::unix::fs::symlink(link_target, at).map_err(err)?;
    Ok(true)
}

/// Take `only` (names as `list` gives them; empty: everything) out of the archive into `dest`.
pub fn extract(path: &Path, dest: &Path, only: &[String], cb: Cb<'_>) -> Result<Summary, String> {
    let format = Format::detect(path).ok_or("Not an archive this program opens")?;
    fs::create_dir_all(dest).map_err(err)?;
    let mut sum = Summary::default();
    let mut prog = Progress::default();
    // one entry of any archive: where it goes, what it is
    let mut handle = |name: &str, kind: Kind, reader: &mut dyn Read, mode: Option<u32>, sum: &mut Summary, prog: &mut Progress, cb: Cb<'_>| -> Result<(), String> {
        if !wanted(only, name) {
            return Ok(());
        }
        let Some(to) = target(dest, name) else {
            sum.skipped += 1;
            return Ok(());
        };
        prog.current = name.to_string();
        prog.entries += 1;
        if make_parent(dest, &to).is_err() {
            sum.skipped += 1;
            return Ok(());
        }
        match kind {
            Kind::Dir => {
                if !to.is_dir() {
                    if fs::symlink_metadata(&to).is_ok() {
                        sum.skipped += 1;
                        return Ok(());
                    }
                    fs::create_dir(&to).map_err(err)?;
                }
                sum.dirs += 1;
            }
            Kind::File => {
                sum.bytes += copy_out(reader, &to, mode, prog, cb)?;
                sum.files += 1;
            }
            Kind::Link(t) => {
                if make_link(dest, &to, &t)? {
                    sum.files += 1;
                } else {
                    sum.skipped += 1;
                }
            }
        }
        if !cb(prog) {
            return Err(CANCELLED.into());
        }
        Ok(())
    };
    match format {
        Format::Zip => {
            let mut z = zip::ZipArchive::new(BufReader::new(File::open(path).map_err(err)?)).map_err(err)?;
            for i in 0..z.len() {
                let mut f = z.by_index(i).map_err(err)?;
                let name = f.name().to_string();
                let mode = f.unix_mode();
                let kind = if f.is_dir() {
                    Kind::Dir
                } else if mode.is_some_and(|m| m & 0o170000 == 0o120000) {
                    let mut t = String::new();
                    f.read_to_string(&mut t).map_err(err)?;
                    Kind::Link(t)
                } else {
                    Kind::File
                };
                handle(&name, kind, &mut f, mode, &mut sum, &mut prog, cb)?;
            }
        }
        Format::SevenZ => {
            let mut r = sevenz_rust2::SevenZReader::open(path, sevenz_rust2::Password::empty()).map_err(err)?;
            let mut failure: Option<String> = None;
            let res = r.for_each_entries(|e, reader| {
                let kind = if e.is_directory { Kind::Dir } else { Kind::File };
                match handle(&e.name, kind, reader, None, &mut sum, &mut prog, cb) {
                    Ok(()) => {
                        // an entry nobody read still has to be passed over
                        let _ = io::copy(reader, &mut io::sink());
                        Ok(true)
                    }
                    Err(m) => {
                        failure = Some(m);
                        Ok(false)
                    }
                }
            });
            if let Some(m) = failure {
                return Err(m);
            }
            res.map_err(err)?;
        }
        f if f.single() => {
            let name = single_name(path);
            let mut r = single_reader(path, f)?;
            handle(&name, Kind::File, &mut r, None, &mut sum, &mut prog, cb)?;
        }
        f => {
            let mut a = tar::Archive::new(tar_reader(path, f)?);
            for e in a.entries().map_err(err)? {
                let mut e = e.map_err(err)?;
                let name = e.path().map_err(err)?.to_string_lossy().to_string();
                let mode = e.header().mode().ok();
                let kind = match e.header().entry_type() {
                    t if t.is_dir() => Kind::Dir,
                    t if t.is_file() => Kind::File,
                    t if t.is_symlink() => match e.link_name().map_err(err)? {
                        Some(l) => Kind::Link(l.to_string_lossy().to_string()),
                        None => {
                            sum.skipped += 1;
                            continue;
                        }
                    },
                    _ => {
                        // hard links, devices, pipes: not made
                        sum.skipped += 1;
                        continue;
                    }
                };
                handle(&name, kind, &mut e, mode, &mut sum, &mut prog, cb)?;
            }
        }
    }
    Ok(sum)
}

enum Kind {
    Dir,
    File,
    Link(String),
}

// ------------------------------------------------------------------ making

struct Item {
    path: PathBuf,
    name: String,
    kind: ItemKind,
}

enum ItemKind {
    Dir,
    File(u64),
    Link(String),
}

/// The files and folders to put in, each under its own name (folders with what is inside), in a steady order.
fn gather(sources: &[PathBuf], skip: &Path) -> Result<Vec<Item>, String> {
    fn walk(path: &Path, name: String, skip: &Path, out: &mut Vec<Item>) -> Result<(), String> {
        if path == skip {
            return Ok(());
        }
        let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            let t = fs::read_link(path).map_err(err)?;
            out.push(Item { path: path.to_path_buf(), name, kind: ItemKind::Link(t.to_string_lossy().to_string()) });
        } else if meta.is_dir() {
            out.push(Item { path: path.to_path_buf(), name: name.clone(), kind: ItemKind::Dir });
            let mut kids: Vec<_> = fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?.flatten().collect();
            kids.sort_by_key(|k| k.file_name());
            for k in kids {
                walk(&k.path(), format!("{name}/{}", k.file_name().to_string_lossy()), skip, out)?;
            }
        } else if meta.is_file() {
            out.push(Item { path: path.to_path_buf(), name, kind: ItemKind::File(meta.len()) });
        }
        Ok(())
    }
    let mut out = Vec::new();
    for s in sources {
        let name = s.file_name().map(|n| n.to_string_lossy().to_string()).ok_or("A source has no name")?;
        walk(s, name, skip, &mut out)?;
    }
    Ok(out)
}

/// A name for the archive of these: the one thing's own name, or "Archive".
pub fn default_name(sources: &[PathBuf]) -> String {
    match sources {
        [one] => one.file_stem().or_else(|| one.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Archive".into()),
        _ => "Archive".into(),
    }
}

fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).map(|m| m.permissions().mode() & 0o777).unwrap_or(0o644)
}

/// Make the archive `out` (in `format`) of `sources`. A job that fails or is stopped leaves no half-made file.
pub fn create(format: Format, out: &Path, sources: &[PathBuf], cb: Cb<'_>) -> Result<Summary, String> {
    if format.single() {
        return Err("A single compressed file is made of one file, not an archive".into());
    }
    let items = gather(sources, out)?;
    if items.is_empty() {
        return Err("Nothing to put in".into());
    }
    let result = create_inner(format, out, &items, cb);
    if result.is_err() {
        let _ = fs::remove_file(out);
    }
    result
}

fn create_inner(format: Format, out: &Path, items: &[Item], cb: Cb<'_>) -> Result<Summary, String> {
    let mut sum = Summary::default();
    let mut prog = Progress::default();
    let file = File::create(out).map_err(err)?;
    let mut step = |name: &str, bytes: u64, prog: &mut Progress, sum: &mut Summary, dir: bool| -> Result<(), String> {
        prog.current = name.to_string();
        prog.entries += 1;
        prog.bytes += bytes;
        sum.bytes += bytes;
        if dir {
            sum.dirs += 1;
        } else {
            sum.files += 1;
        }
        if cb(prog) {
            Ok(())
        } else {
            Err(CANCELLED.into())
        }
    };
    match format {
        Format::Zip => {
            let mut z = zip::ZipWriter::new(BufWriter::new(file));
            for it in items {
                let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).unix_permissions(mode_of(&it.path)).large_file(true);
                match &it.kind {
                    ItemKind::Dir => {
                        z.add_directory(format!("{}/", it.name), opts).map_err(err)?;
                        step(&it.name, 0, &mut prog, &mut sum, true)?;
                    }
                    ItemKind::File(len) => {
                        z.start_file(&it.name, opts).map_err(err)?;
                        io::copy(&mut BufReader::new(File::open(&it.path).map_err(err)?), &mut z).map_err(err)?;
                        step(&it.name, *len, &mut prog, &mut sum, false)?;
                    }
                    ItemKind::Link(t) => {
                        z.add_symlink(&it.name, t, opts).map_err(err)?;
                        step(&it.name, 0, &mut prog, &mut sum, false)?;
                    }
                }
            }
            z.finish().map_err(err)?.flush().map_err(err)?;
        }
        Format::SevenZ => {
            let mut w = sevenz_rust2::SevenZWriter::new(file).map_err(err)?;
            for it in items {
                match &it.kind {
                    ItemKind::Dir => {
                        w.push_archive_entry::<File>(sevenz_rust2::SevenZArchiveEntry::new_folder(&it.name), None).map_err(err)?;
                        step(&it.name, 0, &mut prog, &mut sum, true)?;
                    }
                    ItemKind::File(len) => {
                        w.push_archive_entry(sevenz_rust2::SevenZArchiveEntry::new_file(&it.name), Some(File::open(&it.path).map_err(err)?)).map_err(err)?;
                        step(&it.name, *len, &mut prog, &mut sum, false)?;
                    }
                    // 7z keeps no links: the file or folder they point to is not followed either
                    ItemKind::Link(_) => sum.skipped += 1,
                }
            }
            w.finish().map_err(err)?;
        }
        f => {
            let writer: Box<dyn Write> = match f {
                Format::Tar => Box::new(BufWriter::new(file)),
                Format::TarGz => Box::new(flate2::write::GzEncoder::new(BufWriter::new(file), flate2::Compression::default())),
                Format::TarBz2 => Box::new(bzip2::write::BzEncoder::new(BufWriter::new(file), bzip2::Compression::default())),
                Format::TarXz => Box::new(xz2::write::XzEncoder::new(BufWriter::new(file), 6)),
                Format::TarZst => Box::new(zstd::stream::write::Encoder::new(BufWriter::new(file), 3).map_err(err)?.auto_finish()),
                _ => return Err("not a tar".into()),
            };
            let mut b = tar::Builder::new(writer);
            b.follow_symlinks(false);
            for it in items {
                match &it.kind {
                    ItemKind::Dir => {
                        b.append_dir(&it.name, &it.path).map_err(err)?;
                        step(&it.name, 0, &mut prog, &mut sum, true)?;
                    }
                    ItemKind::File(len) => {
                        b.append_path_with_name(&it.path, &it.name).map_err(err)?;
                        step(&it.name, *len, &mut prog, &mut sum, false)?;
                    }
                    ItemKind::Link(_) => {
                        b.append_path_with_name(&it.path, &it.name).map_err(err)?;
                        step(&it.name, 0, &mut prog, &mut sum, false)?;
                    }
                }
            }
            // closing writes the end of the stream (the compressors finish when they are dropped)
            let mut w = b.into_inner().map_err(err)?;
            w.flush().map_err(err)?;
        }
    }
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("memrium-arc-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let src = root.join("src");
        fs::create_dir_all(src.join("docs/deep")).unwrap();
        fs::create_dir_all(src.join("empty")).unwrap();
        fs::write(src.join("hello.txt"), "hello archive\n").unwrap();
        fs::write(src.join("docs/deep/ґанок.txt"), "unicode name").unwrap();
        fs::write(src.join("big.bin"), vec![7u8; 300_000]).unwrap();
        fs::write(src.join("run.sh"), "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(src.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        (root, src)
    }

    fn go(_: &Progress) -> bool {
        true
    }

    #[test]
    fn every_format_makes_lists_and_gives_back_the_same_files() {
        for format in CREATE {
            let (root, src) = tree(format.extension());
            let out = root.join(format!("a.{}", format.extension()));
            let made = create(format, &out, &[src.clone()], &mut go).unwrap_or_else(|e| panic!("{}: {e}", format.label()));
            assert!(made.files >= 4, "{} made {made:?}", format.label());
            assert_eq!(Format::detect(&out), Some(format));
            let names: Vec<String> = list(&out).unwrap_or_else(|e| panic!("{}: {e}", format.label())).into_iter().map(|e| e.path).collect();
            for want in ["src", "src/hello.txt", "src/docs", "src/docs/deep/ґанок.txt", "src/empty"] {
                assert!(names.iter().any(|n| n == want), "{} lacks {want}: {names:?}", format.label());
            }
            let dest = root.join("out");
            let got = extract(&out, &dest, &[], &mut go).unwrap_or_else(|e| panic!("{}: {e}", format.label()));
            assert_eq!(got.skipped, 0, "{}", format.label());
            assert_eq!(fs::read_to_string(dest.join("src/hello.txt")).unwrap(), "hello archive\n", "{}", format.label());
            assert_eq!(fs::read(dest.join("src/big.bin")).unwrap().len(), 300_000, "{}", format.label());
            assert_eq!(fs::read_to_string(dest.join("src/docs/deep/ґанок.txt")).unwrap(), "unicode name");
            assert!(dest.join("src/empty").is_dir(), "{}", format.label());
            if format != Format::SevenZ {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(fs::metadata(dest.join("src/run.sh")).unwrap().permissions().mode() & 0o111, 0o111, "{} keeps the executable bit", format.label());
            }
            let _ = fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn a_part_of_an_archive_can_be_taken_alone() {
        let (root, src) = tree("part");
        let out = root.join("a.tar.gz");
        create(Format::TarGz, &out, &[src], &mut go).unwrap();
        let dest = root.join("out");
        extract(&out, &dest, &["src/docs".to_string()], &mut go).unwrap();
        assert!(dest.join("src/docs/deep/ґанок.txt").exists());
        assert!(!dest.join("src/hello.txt").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn names_that_climb_out_are_left_out() {
        let (root, _) = tree("slip");
        // a zip with names that go up, and an absolute one
        let out = root.join("evil.zip");
        {
            let mut z = zip::ZipWriter::new(File::create(&out).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            for name in ["../escaped.txt", "/abs.txt", "ok/fine.txt", "a/../../b.txt"] {
                z.start_file(name, o).unwrap();
                z.write_all(b"x").unwrap();
            }
            z.finish().unwrap();
        }
        let dest = root.join("out");
        let sum = extract(&out, &dest, &[], &mut go).unwrap();
        assert_eq!((sum.files, sum.skipped), (1, 3));
        assert!(dest.join("ok/fine.txt").exists());
        assert!(!root.join("escaped.txt").exists() && !root.join("abs.txt").exists() && !root.join("b.txt").exists());
        // and a tar whose link points out of the folder, followed by a file written through it
        let tarball = root.join("evil.tar");
        {
            let mut b = tar::Builder::new(File::create(&tarball).unwrap());
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            b.append_link(&mut h, "link", "../../outside").unwrap();
            let mut h2 = tar::Header::new_gnu();
            h2.set_size(1);
            h2.set_mode(0o644);
            b.append_data(&mut h2, "link/pwned.txt", &b"x"[..]).unwrap();
            let mut h3 = tar::Header::new_gnu();
            h3.set_entry_type(tar::EntryType::Symlink);
            h3.set_size(0);
            b.append_link(&mut h3, "inside", "ok/fine.txt").unwrap();
            b.finish().unwrap();
        }
        let dest2 = root.join("out2");
        let sum2 = extract(&tarball, &dest2, &[], &mut go).unwrap();
        // the link that leaves the folder is not made (what follows it becomes a plain folder), and nothing lands outside
        assert!(!fs::symlink_metadata(dest2.join("link")).is_ok_and(|m| m.file_type().is_symlink()), "a link that leaves the folder is not made");
        assert!(!root.join("pwned.txt").exists() && !root.parent().unwrap().join("pwned.txt").exists());
        assert!(fs::symlink_metadata(dest2.join("inside")).is_ok(), "a link that stays inside is made");
        assert_eq!(sum2.skipped, 1, "only the link that leaves the folder is skipped");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn single_compressed_files_open_and_stop_on_request() {
        let (root, src) = tree("single");
        let gz = root.join("hello.txt.gz");
        {
            let mut e = flate2::write::GzEncoder::new(File::create(&gz).unwrap(), flate2::Compression::default());
            e.write_all(&fs::read(src.join("hello.txt")).unwrap()).unwrap();
            e.finish().unwrap();
        }
        assert_eq!(list(&gz).unwrap()[0].path, "hello.txt");
        let dest = root.join("o");
        extract(&gz, &dest, &[], &mut go).unwrap();
        assert_eq!(fs::read_to_string(dest.join("hello.txt")).unwrap(), "hello archive\\n".replace("\\n", "\n"));
        // stopping leaves no half-made archive
        let out = root.join("stopped.zip");
        let mut n = 0;
        let r = create(Format::Zip, &out, &[src], &mut |_: &Progress| {
            n += 1;
            n < 2
        });
        assert_eq!(r.unwrap_err(), CANCELLED);
        assert!(!out.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn formats_come_from_names() {
        for (n, f) in [("a.tar.gz", Format::TarGz), ("A.TGZ", Format::TarGz), ("a.tar.xz", Format::TarXz), ("a.tar", Format::Tar), ("a.zip", Format::Zip), ("a.7z", Format::SevenZ), ("a.gz", Format::Gz), ("a.tar.zst", Format::TarZst), ("a.tbz2", Format::TarBz2)] {
            assert_eq!(Format::detect(Path::new(n)), Some(f), "{n}");
        }
        assert_eq!(Format::detect(Path::new("a.txt")), None);
        assert_eq!(default_name(&[PathBuf::from("/x/photos")]), "photos");
        assert_eq!(default_name(&[PathBuf::from("/x/a"), PathBuf::from("/x/b")]), "Archive");
        assert_eq!(unix_time(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(unix_time(2024, 3, 1, 12, 0, 0), 1709294400);
    }
}
