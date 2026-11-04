//! Text in pictures (the image viewer's Live Text): PaddleOCR PP-OCRv5 on onnxruntime. A small detector (DB) finds the
//! lines of text, the East Slavic recogniser (Ukrainian, Russian, Belarusian, with the Latin letters and digits) reads
//! each one, and every character keeps where it is on the line, so text can be selected by dragging over it. The
//! models (about 13 MB) are fetched on first use into `~/.local/share/memrium/models/ppocr-eslav`
//! (`MEMRIUM_OCR_DIR` points elsewhere). Needs onnxruntime (see `cutout::preload`).

use ort::session::Session;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const DET_URL: &str = "https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_det_onnx/resolve/main/inference.onnx";
const REC_URL: &str = "https://huggingface.co/PaddlePaddle/eslav_PP-OCRv5_mobile_rec_onnx/resolve/main/inference.onnx";
const DICT_URL: &str = "https://huggingface.co/PaddlePaddle/eslav_PP-OCRv5_mobile_rec_onnx/resolve/main/inference.yml";
const FILES: [(&str, &str, u64); 3] = [("det.onnx", DET_URL, 4_826_518), ("rec.onnx", REC_URL, 7_887_627), ("rec.yml", DICT_URL, 4_538)];

/// The detector looks at the picture with its longer side this long (a multiple of 32).
const DET_SIDE: f32 = 960.0;
/// A pixel of the detector's map is text above this; a found region is kept when its mean is above BOX_MIN.
const DET_THRESH: f32 = 0.3;
const BOX_MIN: f32 = 0.6;
/// How far a found region is grown back out (the detector marks the middle of the letters).
const UNCLIP: f32 = 1.5;
/// The recogniser reads a line scaled to this height.
const REC_H: usize = 48;
/// Lines read with less confidence than this are left out.
const MIN_SCORE: f32 = 0.5;

/// Where the models are kept.
pub fn model_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("MEMRIUM_OCR_DIR") {
        return PathBuf::from(d);
    }
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    data.join("memrium/models/ppocr-eslav")
}

/// Are the models there, whole?
pub fn models_present() -> bool {
    let dir = model_dir();
    FILES.iter().all(|(name, _, size)| std::fs::metadata(dir.join(name)).is_ok_and(|m| m.len() == *size))
}

/// Fetch the models that are missing (with `curl`; blocks, so call it from a worker thread).
pub fn fetch_models() -> Result<(), String> {
    let dir = model_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, url, size) in FILES {
        let path = dir.join(name);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() == size) {
            continue;
        }
        let part = dir.join(format!("{name}.part"));
        let status = std::process::Command::new("curl").args(["-fsSL", "--retry", "2", "--connect-timeout", "20", "-o"]).arg(&part).arg(url).status().map_err(|e| format!("curl: {e}"))?;
        let got = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if !status.success() || got != size {
            let _ = std::fs::remove_file(&part);
            return Err(format!("could not download {name} ({got} of {size} bytes)"));
        }
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// One line of text found in a picture.
#[derive(Clone, Debug)]
pub struct Line {
    /// where it is (picture px: x, y, w, h)
    pub rect: [f32; 4],
    pub text: String,
    /// each character's left and right edge (picture px), in the order of `text.chars()`
    pub spans: Vec<[f32; 2]>,
    pub score: f32,
}

impl Line {
    /// The character nearest to the picture x `x`, as a caret position: 0 before the first character, `len` after the
    /// last.
    pub fn caret_at(&self, x: f32) -> usize {
        self.spans.iter().position(|s| x < (s[0] + s[1]) / 2.0).unwrap_or(self.spans.len())
    }
}

/// The detector and the recogniser, ready to run.
pub struct Reader {
    det: Mutex<Session>,
    rec: Mutex<Session>,
    /// the recogniser's characters: index 0 is CTC's blank, then the dictionary, then a space
    chars: Vec<String>,
}

fn err(e: impl std::fmt::Debug) -> String {
    format!("{e:?}")
}

fn debug() -> bool {
    std::env::var_os("MEMRIUM_OCR_DEBUG").is_some()
}

/// The `character_dict` list of a PaddleOCR `inference.yml` (one YAML string per line, maybe quoted).
fn parse_dict(yml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in yml.lines() {
        if line.trim_end() == "  character_dict:" {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        let Some(v) = line.strip_prefix("  - ") else { break };
        let v = if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
            v[1..v.len() - 1].replace("''", "'")
        } else if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
            v[1..v.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
        } else {
            v.to_string()
        };
        out.push(v);
    }
    out
}

impl Reader {
    pub fn load(dir: &Path) -> Result<Reader, String> {
        if !crate::cutout::onnxruntime() {
            return Err("reading text needs onnxruntime".into());
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).clamp(2, 6);
        let open = |name: &str| -> Result<Session, String> { Session::builder().map_err(err)?.with_intra_threads(threads).map_err(err)?.commit_from_file(dir.join(name)).map_err(err) };
        let dict = parse_dict(&std::fs::read_to_string(dir.join("rec.yml")).map_err(err)?);
        if dict.is_empty() {
            return Err("the recogniser's dictionary is empty".into());
        }
        let mut chars = vec![String::new()];
        chars.extend(dict);
        chars.push(" ".into());
        Ok(Reader { det: Mutex::new(open("det.onnx")?), rec: Mutex::new(open("rec.onnx")?), chars })
    }

    /// The lines of text in a picture (`rgba` straight RGBA, `w * h * 4` bytes), in reading order.
    pub fn read(&self, rgba: &[u8], w: usize, h: usize) -> Result<Vec<Line>, String> {
        let t = std::time::Instant::now();
        let boxes = self.detect(rgba, w, h)?;
        let found = boxes.len();
        let mut lines: Vec<Line> = Vec::new();
        for b in boxes {
            if let Some(l) = self.recognise(rgba, w, h, b)? {
                lines.push(l);
            }
        }
        // reading order: by rows (lines whose middles are within half a line of each other share one), left to right
        lines.sort_by(|a, b| {
            let (ay, by) = (a.rect[1] + a.rect[3] / 2.0, b.rect[1] + b.rect[3] / 2.0);
            if (ay - by).abs() < a.rect[3].min(b.rect[3]) / 2.0 {
                a.rect[0].total_cmp(&b.rect[0])
            } else {
                ay.total_cmp(&by)
            }
        });
        if debug() {
            eprintln!("ocr: {found} regions, {} lines read in {:?}", lines.len(), t.elapsed());
        }
        Ok(lines)
    }

    /// Where the text is: boxes (picture px) round the regions the detector marks.
    fn detect(&self, rgba: &[u8], w: usize, h: usize) -> Result<Vec<[f32; 4]>, String> {
        let k = DET_SIDE / w.max(h) as f32;
        let dw = (((w as f32 * k) / 32.0).round() as usize).max(1) * 32;
        let dh = (((h as f32 * k) / 32.0).round() as usize).max(1) * 32;
        let view = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(w as u32, h as u32, rgba).ok_or("bad picture size")?;
        let small = image::imageops::resize(&view, dw as u32, dh as u32, image::imageops::FilterType::Triangle);
        // BGR, as the model was trained on, each channel normalised with the ImageNet numbers in that order
        let (mean, std) = ([0.485f32, 0.456, 0.406], [0.229f32, 0.224, 0.225]);
        let mut input = vec![0f32; 3 * dw * dh];
        for (x, y, p) in small.enumerate_pixels() {
            let a = p[3] as f32 / 255.0;
            // (see-through parts are read as white paper)
            let bgr = [p[2], p[1], p[0]].map(|c| (c as f32 * a + 255.0 * (1.0 - a)) / 255.0);
            for c in 0..3 {
                input[c * dw * dh + y as usize * dw + x as usize] = (bgr[c] - mean[c]) / std[c];
            }
        }
        let map: Vec<f32> = {
            let x = ort::value::Tensor::from_array(([1usize, 3, dh, dw], input)).map_err(err)?;
            let mut s = self.det.lock().map_err(err)?;
            let out = s.run(ort::inputs![x]).map_err(err)?;
            out[0].try_extract_tensor::<f32>().map_err(err)?.1.to_vec()
        };
        if map.len() != dw * dh {
            return Err(format!("the detector gave {} values", map.len()));
        }
        // the regions: 4-connected runs of text pixels
        let mut seen = vec![false; dw * dh];
        let mut boxes = Vec::new();
        let (sx, sy) = (w as f32 / dw as f32, h as f32 / dh as f32);
        for start in 0..dw * dh {
            if seen[start] || map[start] <= DET_THRESH {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (dw, dh, 0, 0);
            let (mut sum, mut n) = (0f32, 0usize);
            let mut stack = vec![start];
            seen[start] = true;
            while let Some(i) = stack.pop() {
                let (x, y) = (i % dw, i / dw);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
                sum += map[i];
                n += 1;
                for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                    if nx < dw && ny < dh {
                        let j = ny * dw + nx;
                        if !seen[j] && map[j] > DET_THRESH {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
            let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
            if sum / (n as f32) < BOX_MIN || bw.min(bh) < 3.0 {
                continue;
            }
            // grow it back out by area x ratio / perimeter, as the model's own post-processing does
            let d = bw * bh * UNCLIP / (2.0 * (bw + bh));
            let (fx0, fy0) = ((x0 as f32 - d).max(0.0), (y0 as f32 - d).max(0.0));
            let (fx1, fy1) = ((x1 as f32 + 1.0 + d).min(dw as f32), (y1 as f32 + 1.0 + d).min(dh as f32));
            boxes.push([fx0 * sx, fy0 * sy, (fx1 - fx0) * sx, (fy1 - fy0) * sy]);
        }
        Ok(boxes)
    }

    /// Read one box: its text, and where each character is.
    fn recognise(&self, rgba: &[u8], w: usize, h: usize, b: [f32; 4]) -> Result<Option<Line>, String> {
        let (x0, y0) = (b[0].floor().max(0.0) as u32, b[1].floor().max(0.0) as u32);
        let (x1, y1) = (((b[0] + b[2]).ceil() as u32).min(w as u32), ((b[1] + b[3]).ceil() as u32).min(h as u32));
        if x1 <= x0 + 2 || y1 <= y0 + 2 {
            return Ok(None);
        }
        let (cw, ch) = (x1 - x0, y1 - y0);
        // a tall, narrow box is text running down the page: not read here
        if ch as f32 > cw as f32 * 1.5 && cw < 3 * REC_H as u32 {
            return Ok(None);
        }
        let mut crop = image::RgbaImage::new(cw, ch);
        for y in 0..ch {
            let row = ((y0 + y) as usize * w + x0 as usize) * 4;
            crop.as_mut()[(y * cw * 4) as usize..((y + 1) * cw * 4) as usize].copy_from_slice(&rgba[row..row + cw as usize * 4]);
        }
        let rw = ((REC_H as f32 * cw as f32 / ch as f32).round() as usize).clamp(16, 3200);
        let line = image::imageops::resize(&crop, rw as u32, REC_H as u32, image::imageops::FilterType::Triangle);
        let mut input = vec![0f32; 3 * REC_H * rw];
        for (x, y, p) in line.enumerate_pixels() {
            let a = p[3] as f32 / 255.0;
            let bgr = [p[2], p[1], p[0]].map(|c| (c as f32 * a + 255.0 * (1.0 - a)) / 255.0);
            for c in 0..3 {
                input[c * REC_H * rw + y as usize * rw + x as usize] = (bgr[c] - 0.5) / 0.5;
            }
        }
        let (shape, probs): (Vec<i64>, Vec<f32>) = {
            let x = ort::value::Tensor::from_array(([1usize, 3, REC_H, rw], input)).map_err(err)?;
            let mut s = self.rec.lock().map_err(err)?;
            let out = s.run(ort::inputs![x]).map_err(err)?;
            let (sh, v) = out[0].try_extract_tensor::<f32>().map_err(err)?;
            (sh.to_vec(), v.to_vec())
        };
        let (steps, classes) = (*shape.get(1).unwrap_or(&0) as usize, *shape.get(2).unwrap_or(&0) as usize);
        if steps == 0 || classes == 0 || probs.len() != steps * classes {
            return Ok(None);
        }
        // CTC: the likeliest class at each step, repeats merged, blanks dropped
        let mut text = String::new();
        let mut centres: Vec<f32> = Vec::new();
        let (mut conf, mut emitted) = (0f32, 0usize);
        let mut prev = 0usize;
        for t in 0..steps {
            let row = &probs[t * classes..(t + 1) * classes];
            let (k, p) = row.iter().enumerate().fold((0, f32::MIN), |m, (i, v)| if *v > m.1 { (i, *v) } else { m });
            if k != 0 && k != prev {
                if let Some(c) = self.chars.get(k) {
                    for ch in c.chars() {
                        text.push(ch);
                        centres.push(x0 as f32 + (t as f32 + 0.5) / steps as f32 * cw as f32);
                    }
                    conf += p;
                    emitted += 1;
                }
            }
            prev = k;
        }
        // no spaces at the ends (with their places)
        let all: Vec<char> = text.chars().collect();
        let first = all.iter().position(|c| !c.is_whitespace());
        let Some(first) = first else { return Ok(None) };
        let last = all.iter().rposition(|c| !c.is_whitespace()).unwrap_or(first);
        let text: String = all[first..=last].iter().collect();
        let centres: Vec<f32> = centres[first..=last].to_vec();
        let n = centres.len();
        let score = conf / emitted.max(1) as f32;
        if score < MIN_SCORE {
            return Ok(None);
        }
        // each character from halfway to the one before to halfway to the one after (the ends: the box's edges)
        let mut spans = Vec::with_capacity(n);
        for i in 0..n {
            let left = if i == 0 { x0 as f32 } else { (centres[i - 1] + centres[i]) / 2.0 };
            let right = if i + 1 == n { x1 as f32 } else { (centres[i] + centres[i + 1]) / 2.0 };
            spans.push([left, right]);
        }
        Ok(Some(Line { rect: [x0 as f32, y0 as f32, cw as f32, ch as f32], text, spans, score }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dictionary_is_read_with_its_quotes_undone() {
        let yml = "PostProcess:\n  name: CTCLabelDecode\n  character_dict:\n  - '!'\n  - ''''\n  - a\n  - ї\n  - \\\nsomething: else\n  - x\n";
        assert_eq!(parse_dict(yml), vec!["!", "'", "a", "ї", "\\"]);
    }

    #[test]
    fn the_caret_lands_between_characters() {
        let l = Line { rect: [0.0, 0.0, 30.0, 10.0], text: "abc".into(), spans: vec![[0.0, 10.0], [10.0, 20.0], [20.0, 30.0]], score: 1.0 };
        assert_eq!(l.caret_at(-5.0), 0);
        assert_eq!(l.caret_at(4.0), 0);
        assert_eq!(l.caret_at(6.0), 1);
        assert_eq!(l.caret_at(26.0), 3);
    }
}
