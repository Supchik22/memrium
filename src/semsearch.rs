//! Search by meaning (opt-in, off unless turned on in Settings > Privacy): pictures are found by what is in them
//! ("cat on a sofa", "кіт на дивані") and screenshots and documents by the words in them.
//!
//! * Pictures: CLIP ViT-B/32 (the image tower, int8) gives each picture a vector; the query is put in the same space
//!   by the multilingual text tower distilled from CLIP's (DistilBERT, 50+ languages, Ukrainian among them), mean-pooled
//!   and projected 768 -> 512 by its dense layer. Both run on onnxruntime (see `cutout::onnxruntime`).
//! * Screenshots and PNGs: their text, read by the OCR of Live Text (`ocr`), is searched word by word.
//! * Documents (txt, md, pdf through `pdftotext`): their text is searched by words, and its start by meaning (the
//!   text tower embeds it too).
//!
//! Nothing runs while it is off: the models (about 225 MB) are fetched only when it is turned on, and the index is
//! built by `memrium_semindex`, which the shell starts on mains power while the computer is idle. The index lives in
//! `~/.cache/memrium/semantic/index.bin`; `status` beside it says how far it got.

use ort::session::Session;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

const VISION_URL: &str = "https://huggingface.co/Xenova/clip-vit-base-patch32/resolve/main/onnx/vision_model_quantized.onnx";
const TEXT_URL: &str = "https://huggingface.co/sentence-transformers/clip-ViT-B-32-multilingual-v1/resolve/main/onnx/model_quint8_avx2.onnx";
const DENSE_URL: &str = "https://huggingface.co/sentence-transformers/clip-ViT-B-32-multilingual-v1/resolve/main/2_Dense/model.safetensors";
const VOCAB_URL: &str = "https://huggingface.co/sentence-transformers/clip-ViT-B-32-multilingual-v1/resolve/main/vocab.txt";
const FILES: [(&str, &str, u64); 4] = [("vision.onnx", VISION_URL, 89_117_001), ("text.onnx", TEXT_URL, 135_377_779), ("dense.safetensors", DENSE_URL, 1_572_984), ("vocab.txt", VOCAB_URL, 995_526)];

/// The length of a vector (CLIP ViT-B/32's joint space).
pub const DIM: usize = 512;
/// The text tower reads at most this many word pieces.
const MAX_TOKENS: usize = 128;

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// Where the models are kept (`MEMRIUM_CLIP_DIR` points elsewhere).
pub fn model_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("MEMRIUM_CLIP_DIR") {
        return PathBuf::from(d);
    }
    data_home().join("memrium/models/clip-ml")
}

/// Where the index and its status are kept (`MEMRIUM_SEMANTIC_DIR` points elsewhere).
pub fn index_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("MEMRIUM_SEMANTIC_DIR") {
        return PathBuf::from(d);
    }
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache"));
    cache.join("memrium/semantic")
}

pub fn index_path() -> PathBuf {
    index_dir().join("index.bin")
}

/// One line saying how far the index got ("Indexing 120 of 15000", "Ready: 15000 files"), for Settings.
pub fn status() -> String {
    std::fs::read_to_string(index_dir().join("status")).map(|s| s.trim().to_string()).unwrap_or_default()
}

pub fn set_status(s: &str) {
    let dir = index_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("status"), s);
}

/// Are the models there, whole?
pub fn models_present() -> bool {
    let dir = model_dir();
    FILES.iter().all(|(name, _, size)| std::fs::metadata(dir.join(name)).is_ok_and(|m| m.len() == *size))
}

/// Fetch what is missing (resuming a broken download), reporting progress through `status`.
pub fn fetch_models() -> Result<(), String> {
    let dir = model_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let total: u64 = FILES.iter().map(|f| f.2).sum();
    let mut done = 0;
    for (name, url, size) in FILES {
        let path = dir.join(name);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() == size) {
            done += size;
            continue;
        }
        set_status(&format!("Downloading the models ({} of {} MB)", done / 1_000_000, total / 1_000_000));
        let part = dir.join(format!("{name}.part"));
        let status = std::process::Command::new("curl")
            .args(["-fsSL", "--retry", "8", "--retry-all-errors", "-C", "-", "--connect-timeout", "20", "-o"])
            .arg(&part)
            .arg(url)
            .status()
            .map_err(|e| format!("curl: {e}"))?;
        let got = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if !status.success() || got != size {
            return Err(format!("could not download {name} ({got} of {size} bytes)"));
        }
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
        done += size;
    }
    Ok(())
}

fn err(e: impl std::fmt::Debug) -> String {
    format!("{e:?}")
}

// ------------------------------------------------------------------ the text tower's word pieces

/// BERT's word pieces for the multilingual text tower (cased: no lower-casing, accents kept).
pub struct WordPiece {
    vocab: HashMap<String, i64>,
    unk: i64,
    cls: i64,
    sep: i64,
}

impl WordPiece {
    pub fn from_vocab(text: &str) -> WordPiece {
        let vocab: HashMap<String, i64> = text.lines().enumerate().map(|(i, l)| (l.to_string(), i as i64)).collect();
        let id = |t: &str, d: i64| vocab.get(t).copied().unwrap_or(d);
        WordPiece { unk: id("[UNK]", 100), cls: id("[CLS]", 101), sep: id("[SEP]", 102), vocab }
    }

    /// The ids of `text` with [CLS] and [SEP] round them, at most `MAX_TOKENS` long.
    pub fn encode(&self, text: &str) -> Vec<i64> {
        let mut ids = vec![self.cls];
        'words: for word in basic_split(text) {
            let chars: Vec<char> = word.chars().collect();
            if chars.len() > 100 {
                ids.push(self.unk);
                continue;
            }
            let mut pieces = Vec::new();
            let mut start = 0;
            while start < chars.len() {
                let mut end = chars.len();
                let mut found = None;
                while start < end {
                    let mut s: String = chars[start..end].iter().collect();
                    if start > 0 {
                        s.insert_str(0, "##");
                    }
                    if let Some(&id) = self.vocab.get(&s) {
                        found = Some(id);
                        break;
                    }
                    end -= 1;
                }
                match found {
                    Some(id) => {
                        pieces.push(id);
                        start = end;
                    }
                    None => {
                        ids.push(self.unk);
                        continue 'words;
                    }
                }
            }
            ids.extend(pieces);
            if ids.len() >= MAX_TOKENS - 1 {
                break;
            }
        }
        ids.truncate(MAX_TOKENS - 1);
        ids.push(self.sep);
        ids
    }
}

/// BERT's basic split: control characters dropped, words split on spaces, every punctuation mark and CJK character a
/// word of its own.
fn basic_split(text: &str) -> Vec<String> {
    let cjk = |c: char| matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x20000..=0x2A6DF | 0xF900..=0xFAFF);
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c == '\u{0}' || c == '\u{fffd}' || (c.is_control() && !c.is_whitespace()) {
            continue;
        }
        if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else if cjk(c) || c.is_ascii_punctuation() || (!c.is_alphanumeric() && !c.is_whitespace()) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ------------------------------------------------------------------ the models

/// The two towers of CLIP: pictures and (multilingual) text, into one space.
pub struct Clip {
    vision: Option<Mutex<Session>>,
    text: Mutex<Session>,
    /// the text tower's projection 768 -> 512 (row-major, 512 rows)
    dense: Vec<f32>,
    pieces: WordPiece,
}

impl Clip {
    /// Load the models (`vision`: the picture tower too; a search needs only the text one).
    pub fn load(vision: bool) -> Result<Clip, String> {
        if !crate::cutout::onnxruntime() {
            return Err("search by meaning needs onnxruntime".into());
        }
        let dir = model_dir();
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).clamp(1, 4);
        let open = |name: &str| -> Result<Session, String> { Session::builder().map_err(err)?.with_intra_threads(threads).map_err(err)?.commit_from_file(dir.join(name)).map_err(err) };
        let dense = read_dense(&std::fs::read(dir.join("dense.safetensors")).map_err(err)?)?;
        let pieces = WordPiece::from_vocab(&std::fs::read_to_string(dir.join("vocab.txt")).map_err(err)?);
        Ok(Clip { vision: if vision { Some(Mutex::new(open("vision.onnx")?)) } else { None }, text: Mutex::new(open("text.onnx")?), dense, pieces })
    }

    /// The vector of a piece of text (unit length).
    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>, String> {
        let ids = self.pieces.encode(text);
        let n = ids.len();
        let mask = vec![1i64; n];
        let hidden: Vec<f32> = {
            let x = ort::value::Tensor::from_array(([1usize, n], ids)).map_err(err)?;
            let m = ort::value::Tensor::from_array(([1usize, n], mask)).map_err(err)?;
            let mut s = self.text.lock().map_err(err)?;
            let out = s.run(ort::inputs!["input_ids" => x, "attention_mask" => m]).map_err(err)?;
            let (_, v) = out[0].try_extract_tensor::<f32>().map_err(err)?;
            v.to_vec()
        };
        if hidden.len() != n * 768 {
            return Err(format!("the text tower gave {} numbers for {n} pieces", hidden.len()));
        }
        // mean of the pieces, then the projection into the joint space
        let mut pooled = [0f32; 768];
        for t in 0..n {
            for (p, h) in pooled.iter_mut().zip(&hidden[t * 768..(t + 1) * 768]) {
                *p += h / n as f32;
            }
        }
        let mut v: Vec<f32> = (0..DIM).map(|i| self.dense[i * 768..(i + 1) * 768].iter().zip(&pooled).map(|(w, p)| w * p).sum()).collect();
        normalise(&mut v);
        Ok(v)
    }

    /// The vector of a picture (straight RGBA, `w * h * 4` bytes; unit length).
    pub fn embed_image(&self, rgba: &[u8], w: u32, h: u32) -> Result<Vec<f32>, String> {
        let Some(vision) = &self.vision else { return Err("the picture tower is not loaded".into()) };
        let input = clip_pixels(rgba, w, h);
        let x = ort::value::Tensor::from_array(([1usize, 3, 224, 224], input)).map_err(err)?;
        let mut s = vision.lock().map_err(err)?;
        let out = s.run(ort::inputs!["pixel_values" => x]).map_err(err)?;
        // the projected vector (image_embeds), whatever order the outputs come in
        for (_, value) in out.iter() {
            if let Ok((shape, data)) = value.try_extract_tensor::<f32>() {
                if shape.len() == 2 && shape[1] as usize == DIM {
                    let mut v = data.to_vec();
                    normalise(&mut v);
                    return Ok(v);
                }
            }
        }
        Err("the picture tower gave no vector".into())
    }
}

/// The weight of the text tower's dense layer from its safetensors file (one F32 matrix, 512 x 768).
fn read_dense(bytes: &[u8]) -> Result<Vec<f32>, String> {
    let n = u64::from_le_bytes(bytes.get(..8).ok_or("short file")?.try_into().map_err(err)?) as usize;
    let header: serde_json::Value = serde_json::from_slice(bytes.get(8..8 + n).ok_or("short header")?).map_err(err)?;
    let (_, t) = header.as_object().and_then(|o| o.iter().find(|(k, _)| k.as_str() != "__metadata__")).ok_or("no tensor")?;
    if t["dtype"] != "F32" || t["shape"] != serde_json::json!([DIM, 768]) {
        return Err(format!("unexpected dense layer {t}"));
    }
    let a = t["data_offsets"][0].as_u64().ok_or("offsets")? as usize + 8 + n;
    let b = t["data_offsets"][1].as_u64().ok_or("offsets")? as usize + 8 + n;
    Ok(bytes.get(a..b).ok_or("short data")?.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

fn normalise(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= n);
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// CLIP's input: the shorter side scaled to 224, the middle 224 x 224 cut out, normalised by CLIP's mean and spread
/// (planar RGB).
fn clip_pixels(rgba: &[u8], w: u32, h: u32) -> Vec<f32> {
    const MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
    const STD: [f32; 3] = [0.268_629_54, 0.261_302_58, 0.275_777_11];
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).unwrap_or_else(|| image::RgbaImage::new(1, 1));
    let s = 224.0 / w.min(h).max(1) as f32;
    let (nw, nh) = (((w as f32 * s).round() as u32).max(224), ((h as f32 * s).round() as u32).max(224));
    let small = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
    let (ox, oy) = ((nw - 224) / 2, (nh - 224) / 2);
    let mut out = vec![0f32; 3 * 224 * 224];
    for y in 0..224u32 {
        for x in 0..224u32 {
            let p = small.get_pixel(ox + x, oy + y);
            for c in 0..3 {
                out[c * 224 * 224 + (y * 224 + x) as usize] = (p[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
    }
    out
}

// ------------------------------------------------------------------ the index

/// What kind of file an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Photo,
    Screenshot,
    Document,
}

/// One indexed file: where it is, when it changed, its vector (pictures: what is in it; documents: what its start
/// says; empty when there is none) and its words (screenshots and documents; lower-case).
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub mtime: u64,
    pub kind: Kind,
    pub vec: Vec<f32>,
    pub text: String,
}

#[derive(Default)]
pub struct Index {
    pub entries: Vec<Entry>,
}

const MAGIC: &[u8; 8] = b"MEMSEM01";

impl Index {
    pub fn load(path: &Path) -> Index {
        let Ok(b) = std::fs::read(path) else { return Index::default() };
        Self::parse(&b).unwrap_or_default()
    }

    fn parse(b: &[u8]) -> Option<Index> {
        if b.get(..8)? != MAGIC {
            return None;
        }
        let mut at = 8;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = b.get(at..at + n)?;
            at += n;
            Some(s)
        };
        let count = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let pl = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
            let path = PathBuf::from(String::from_utf8_lossy(take(pl)?).into_owned());
            let mtime = u64::from_le_bytes(take(8)?.try_into().ok()?);
            let kind = match take(1)?[0] {
                0 => Kind::Photo,
                1 => Kind::Screenshot,
                _ => Kind::Document,
            };
            let dims = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
            let vec = take(dims * 4)?.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            let tl = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
            let text = String::from_utf8_lossy(take(tl)?).into_owned();
            entries.push(Entry { path, mtime, kind, vec, text });
        }
        Some(Index { entries })
    }

    /// Write it whole (to a new file first, so a reader never sees half of it).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut b = Vec::with_capacity(self.entries.len() * (DIM * 4 + 128));
        b.extend_from_slice(MAGIC);
        b.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for e in &self.entries {
            let p = e.path.to_string_lossy();
            b.extend_from_slice(&(p.len() as u32).to_le_bytes());
            b.extend_from_slice(p.as_bytes());
            b.extend_from_slice(&e.mtime.to_le_bytes());
            b.push(match e.kind {
                Kind::Photo => 0,
                Kind::Screenshot => 1,
                Kind::Document => 2,
            });
            b.extend_from_slice(&(e.vec.len() as u16).to_le_bytes());
            for x in &e.vec {
                b.extend_from_slice(&x.to_le_bytes());
            }
            b.extend_from_slice(&(e.text.len() as u32).to_le_bytes());
            b.extend_from_slice(e.text.as_bytes());
        }
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(err)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &b).map_err(err)?;
        std::fs::rename(&tmp, path).map_err(err)
    }

    /// The best matches for a query: its vector (None while the text tower is not there) and its words.
    ///
    /// The words: a screenshot or document whose text has all of them is a strong match. The meaning: what counts is
    /// how far a file stands out from the rest, not its raw score: CLIP's picture scores are small and bunched (0.2 ..
    /// 0.33), and the text tower finds any two texts alike (0.7 .. 0.9), so documents are compared with their common
    /// part (the mean of them all) taken away.
    pub fn search(&self, qvec: Option<&[f32]>, query: &str, limit: usize) -> Vec<Hit> {
        let words: Vec<String> = query.to_lowercase().split_whitespace().filter(|w| w.chars().count() >= 2).map(str::to_string).collect();
        let meaning = qvec.map(|q| self.meaning(q)).unwrap_or_default();
        let mut hits: Vec<Hit> = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            let found = if words.is_empty() || e.text.is_empty() { 0 } else { words.iter().filter(|w| e.text.contains(w.as_str())).count() };
            let by_words = found == words.len() && found > 0;
            let meant = meaning.get(&i).copied();
            let score = if by_words { 1.0 } else { 0.0 } + meant.unwrap_or(0.0);
            if by_words || meant.is_some() {
                let why = if by_words { Why::Words(snippet(&e.text, &words)) } else { Why::Meaning(meant.unwrap_or(0.0)) };
                hits.push(Hit { path: e.path.clone(), kind: e.kind, score, why });
            }
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(limit);
        hits
    }

    /// Which entries the query vector `q` picks out by meaning, and how strongly (0 .. 1).
    fn meaning(&self, q: &[f32]) -> HashMap<usize, f32> {
        let mut out = HashMap::new();
        // pictures: CLIP's score as it is
        let pics: Vec<(usize, f32)> = self.entries.iter().enumerate().filter(|(_, e)| e.kind != Kind::Document && e.vec.len() == DIM).map(|(i, e)| (i, dot(q, &e.vec))).collect();
        out.extend(stand_out(&pics, 0.25, 0.04));
        // documents: with what they all share taken away (needs a few of them to know what that is)
        let docs: Vec<(usize, &Vec<f32>)> = self.entries.iter().enumerate().filter(|(_, e)| e.kind == Kind::Document && e.vec.len() == DIM).map(|(i, e)| (i, &e.vec)).collect();
        if docs.len() >= 3 {
            let mut mu = vec![0f32; DIM];
            for (_, v) in &docs {
                for (m, x) in mu.iter_mut().zip(v.iter()) {
                    *m += x / docs.len() as f32;
                }
            }
            let centre = |v: &[f32]| {
                let mut c: Vec<f32> = v.iter().zip(&mu).map(|(x, m)| x - m).collect();
                normalise(&mut c);
                c
            };
            let qc = centre(q);
            let scored: Vec<(usize, f32)> = docs.iter().map(|(i, v)| (*i, dot(&qc, &centre(v)))).collect();
            out.extend(stand_out(&scored, 0.30, 0.08));
        }
        out
    }
}

/// The scores that stand out: with many, those well above the rest (two spreads over the mean) and near the best;
/// with few, those over `floor` and within `near` of the best.
fn stand_out(scores: &[(usize, f32)], floor: f32, near: f32) -> Vec<(usize, f32)> {
    let best = scores.iter().map(|s| s.1).fold(f32::MIN, f32::max);
    if scores.len() >= 20 {
        let n = scores.len() as f32;
        let mean = scores.iter().map(|s| s.1).sum::<f32>() / n;
        let spread = (scores.iter().map(|s| (s.1 - mean).powi(2)).sum::<f32>() / n).sqrt().max(1e-4);
        scores.iter().filter(|s| (s.1 - mean) / spread >= 2.0 && s.1 >= best - 2.0 * near).copied().collect()
    } else {
        scores.iter().filter(|s| s.1 >= floor && s.1 >= best - near).copied().collect()
    }
}


/// A file found, how well, and why.
#[derive(Clone, Debug)]
pub struct Hit {
    pub path: PathBuf,
    pub kind: Kind,
    pub score: f32,
    pub why: Why,
}

#[derive(Clone, Debug)]
pub enum Why {
    /// its text has the words: a bit of it round the first one
    Words(String),
    /// it looks like / is about the query (cosine in CLIP's space)
    Meaning(f32),
}

/// A short bit of `text` round the first of `words`.
fn snippet(text: &str, words: &[String]) -> String {
    let Some(at) = words.iter().filter_map(|w| text.find(w.as_str())).min() else { return String::new() };
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let ci = chars.iter().position(|(b, _)| *b >= at).unwrap_or(0);
    let from = ci.saturating_sub(18);
    let to = (ci + 42).min(chars.len());
    let s: String = chars[from..to].iter().map(|(_, c)| if c.is_whitespace() { ' ' } else { *c }).collect();
    format!("{}{}{}", if from > 0 { "…" } else { "" }, s.trim(), if to < chars.len() { "…" } else { "" })
}

// ------------------------------------------------------------------ building the index

/// Where pictures and documents are looked for.
pub fn roots() -> Vec<PathBuf> {
    // `MEMRIUM_SEMANTIC_ROOTS` (folders split by `:`) instead, for tests
    if let Some(r) = std::env::var_os("MEMRIUM_SEMANTIC_ROOTS") {
        return std::env::split_paths(&r).filter(|p| p.is_dir()).collect();
    }
    let h = home();
    ["Pictures", "Documents", "Desktop", "Downloads"].iter().map(|d| h.join(d)).filter(|p| p.is_dir()).collect()
}

fn ext(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// What kind of entry this file would be (None: not indexed).
pub fn kind_of(p: &Path) -> Option<Kind> {
    let e = ext(p);
    let name = p.file_name()?.to_string_lossy().to_lowercase();
    match e.as_str() {
        "jpg" | "jpeg" | "png" | "webp" => Some(if name.contains("screenshot") || name.starts_with("знімок") || p.components().any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("screenshots")) { Kind::Screenshot } else { Kind::Photo }),
        "txt" | "md" | "pdf" => Some(Kind::Document),
        _ => None,
    }
}

/// Every file to index under the roots (hidden folders and very small pictures left out).
pub fn walk(roots: &[PathBuf]) -> Vec<(PathBuf, u64, Kind)> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() {
                let Some(kind) = kind_of(&p) else { continue };
                let Ok(md) = e.metadata() else { continue };
                if kind != Kind::Document && md.len() < 8_000 {
                    continue;
                }
                let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                out.push((p, mtime, kind));
            }
        }
    }
    out
}

/// A document's text (the start of it: enough to search).
fn document_text(p: &Path) -> String {
    let raw = if ext(p) == "pdf" {
        std::process::Command::new("pdftotext").args(["-l", "5", "-q"]).arg(p).arg("-").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
    } else {
        let mut b = std::fs::read(p).unwrap_or_default();
        b.truncate(64 * 1024);
        String::from_utf8_lossy(&b).into_owned()
    };
    raw.chars().take(20_000).collect()
}

/// Build or bring the index up to date: new and changed files are added, gone ones dropped; saved every so often,
/// so a stop (`stop` set, or the process killed) loses little. Progress goes to `status`.
pub fn build(stop: &AtomicBool) -> Result<usize, String> {
    if !models_present() {
        fetch_models()?;
    }
    set_status("Loading the models");
    let clip = Clip::load(true)?;
    let ocr = if crate::ocr::models_present() || crate::ocr::fetch_models().is_ok() { crate::ocr::Reader::load(&crate::ocr::model_dir()).ok() } else { None };
    let path = index_path();
    let mut index = Index::load(&path);
    let files = walk(&roots());
    let present: std::collections::HashSet<&Path> = files.iter().map(|f| f.0.as_path()).collect();
    index.entries.retain(|e| present.contains(e.path.as_path()));
    let known: HashMap<PathBuf, u64> = index.entries.iter().map(|e| (e.path.clone(), e.mtime)).collect();
    let todo: Vec<&(PathBuf, u64, Kind)> = files.iter().filter(|(p, m, _)| known.get(p) != Some(m)).collect();
    let total = todo.len();
    let mut since_save = 0;
    for (i, (p, mtime, kind)) in todo.iter().enumerate() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if i % 10 == 0 {
            set_status(&format!("Indexing {} of {} new files ({} done before)", i, total, index.entries.len()));
        }
        let entry = match kind {
            Kind::Document => {
                let text = document_text(p);
                let vec = if text.trim().is_empty() { Vec::new() } else { clip.embed_text(&text.chars().take(600).collect::<String>()).unwrap_or_default() };
                Entry { path: p.clone(), mtime: *mtime, kind: *kind, vec, text: text.to_lowercase() }
            }
            _ => {
                let Ok(img) = image::open(p) else { continue };
                // big photos are scaled down first: CLIP sees 224 px, the OCR wants enough to read
                let img = if img.width().max(img.height()) > 1600 { img.thumbnail(1600, 1600) } else { img };
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                let vec = clip.embed_image(&rgba, w, h).unwrap_or_default();
                let text = match (&ocr, kind) {
                    (Some(r), Kind::Screenshot) => r.read(&rgba, w as usize, h as usize).map(|ls| ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default(),
                    _ => String::new(),
                };
                Entry { path: p.clone(), mtime: *mtime, kind: *kind, vec, text: text.to_lowercase() }
            }
        };
        index.entries.retain(|e| e.path != entry.path);
        index.entries.push(entry);
        since_save += 1;
        if since_save >= 100 {
            index.save(&path)?;
            since_save = 0;
        }
    }
    index.save(&path)?;
    let n = index.entries.len();
    set_status(&if stop.load(Ordering::Relaxed) { format!("Paused: {n} files indexed") } else { format!("Ready: {n} files") });
    Ok(n)
}

// ------------------------------------------------------------------ searching from the shell

/// Answers queries off the shell's thread: the text tower and the index are loaded on the first query, the index again
/// when the indexer has written a new one. `results` gives what was found for the latest query once it is ready;
/// `generation` moves on each time new results are there.
pub struct Searcher {
    tx: Mutex<mpsc::Sender<String>>,
    done: Arc<Mutex<(String, Vec<Hit>)>>,
    generation: Arc<AtomicU64>,
}

impl Searcher {
    pub fn new() -> Arc<Searcher> {
        let (tx, rx) = mpsc::channel::<String>();
        let done = Arc::new(Mutex::new((String::new(), Vec::new())));
        let generation = Arc::new(AtomicU64::new(0));
        let (d, g) = (done.clone(), generation.clone());
        std::thread::Builder::new()
            .name("semantic-search".into())
            .spawn(move || {
                let mut clip: Option<Clip> = None;
                let mut tried = false;
                let (mut index, mut stamp) = (Index::default(), None);
                while let Ok(mut q) = rx.recv() {
                    // only the latest query matters
                    while let Ok(next) = rx.try_recv() {
                        q = next;
                    }
                    let now = std::fs::metadata(index_path()).and_then(|m| m.modified()).ok();
                    if now != stamp {
                        index = Index::load(&index_path());
                        stamp = now;
                    }
                    if clip.is_none() && !tried && models_present() {
                        tried = true;
                        clip = Clip::load(false).map_err(|e| eprintln!("semantic search: {e}")).ok();
                    }
                    let qvec = clip.as_ref().and_then(|c| c.embed_text(&q).ok());
                    let hits = index.search(qvec.as_deref(), &q, 6);
                    if let Ok(mut d) = d.lock() {
                        *d = (q, hits);
                    }
                    g.fetch_add(1, Ordering::Relaxed);
                }
            })
            .ok();
        Arc::new(Searcher { tx: Mutex::new(tx), done, generation })
    }

    /// Ask for `query` (answered in the background).
    pub fn ask(&self, query: &str) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(query.to_string());
        }
    }

    /// What was found for `query`, if that is the latest answer.
    pub fn results(&self, query: &str) -> Option<Vec<Hit>> {
        let d = self.done.lock().ok()?;
        (d.0 == query).then(|| d.1.clone())
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_pieces_split_like_bert() {
        let vocab = "[PAD]\n[UNK]\n[CLS]\n[SEP]\nкіт\nна\nди\n##ва\n##ні\n!\ncat";
        let wp = WordPiece::from_vocab(vocab);
        // кіт=4 на=5 ди ##ва ##ні = 6 7 8, ! = 9, an unknown word = [UNK]
        assert_eq!(wp.encode("кіт на дивані!"), vec![2, 4, 5, 6, 7, 8, 9, 3]);
        assert_eq!(wp.encode("dog cat"), vec![2, 1, 10, 3]);
    }

    #[test]
    fn the_index_round_trips_and_finds_by_words_and_meaning() {
        let mut a = vec![0f32; DIM];
        a[0] = 1.0;
        let mut b = vec![0f32; DIM];
        b[1] = 1.0;
        let index = Index {
            entries: vec![
                Entry { path: "/p/cat.jpg".into(), mtime: 1, kind: Kind::Photo, vec: a.clone(), text: String::new() },
                Entry { path: "/p/sea.jpg".into(), mtime: 2, kind: Kind::Photo, vec: b, text: String::new() },
                Entry { path: "/s/Screenshot_1.png".into(), mtime: 3, kind: Kind::Screenshot, vec: Vec::new(), text: "рахунок за світло: 512 грн".into() },
            ],
        };
        let dir = std::env::temp_dir().join(format!("semsearch-test-{}", std::process::id()));
        let path = dir.join("index.bin");
        index.save(&path).unwrap();
        let back = Index::load(&path);
        assert_eq!(back.entries.len(), 3);
        assert_eq!(back.entries[2].text, "рахунок за світло: 512 грн");
        // by meaning: the query vector close to the cat picture
        let mut q = vec![0f32; DIM];
        q[0] = 0.9;
        q[1] = 0.1;
        normalise(&mut q);
        let hits = back.search(Some(&q), "кіт", 5);
        assert_eq!(hits[0].path, PathBuf::from("/p/cat.jpg"));
        assert!(hits.iter().all(|h| h.path != PathBuf::from("/p/sea.jpg")), "far from the best: left out");
        // by words: the screenshot's text
        let hits = back.search(None, "Рахунок світло", 5);
        assert_eq!(hits.len(), 1);
        assert!(matches!(&hits[0].why, Why::Words(s) if s.contains("рахунок")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn screenshots_are_told_from_photos() {
        assert_eq!(kind_of(Path::new("/h/Pictures/Screenshots/a.png")), Some(Kind::Screenshot));
        assert_eq!(kind_of(Path::new("/h/Pictures/Screenshot_20261109.png")), Some(Kind::Screenshot));
        assert_eq!(kind_of(Path::new("/h/Pictures/IMG_1.JPG")), Some(Kind::Photo));
        assert_eq!(kind_of(Path::new("/h/Documents/a.pdf")), Some(Kind::Document));
        assert_eq!(kind_of(Path::new("/h/a.mp4")), None);
    }
}
