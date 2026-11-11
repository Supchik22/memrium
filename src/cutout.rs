//! Lifting an object out of a picture (the image viewer's Alt + drag): MobileSAM, a small Segment Anything, finds the
//! object under a point. The image encoder runs once per picture (about a second on the CPU, on a worker thread); the
//! mask decoder then answers each point in a few tens of milliseconds. Both are ONNX models run by `tract`, downloaded
//! on first use into `~/.local/share/memrium/models/mobilesam` (about 45 MB; `MEMRIUM_SAM_DIR` points elsewhere).

use image::RgbaImage;
use std::path::{Path, PathBuf};
use tract_onnx::prelude::*;

type Plan = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

/// The side of the square the encoder looks at, and of the mask the decoder gives (a quarter of it).
const SIDE: usize = 1024;
const LOW: usize = 256;
/// SAM's colour normalisation: a pixel of exactly this colour becomes zero, which is what the padding has to be
const MEAN: [f32; 3] = [123.675, 116.28, 103.53];
/// Points the decoder is given each time: the ones in use first, the rest padding (label -1, which SAM ignores).
const POINTS: usize = 6;

const FILES: [(&str, u64); 2] = [("mobile_sam_image_encoder.onnx", 28_157_093), ("sam_mask_decoder_multi.onnx", 16_496_559)];
const BASE_URL: &str = "https://huggingface.co/Acly/MobileSAM/resolve/main/";

/// Where the models are kept.
pub fn model_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("MEMRIUM_SAM_DIR") {
        return PathBuf::from(d);
    }
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    data.join("memrium/models/mobilesam")
}

/// Are both models there, whole?
pub fn models_present() -> bool {
    let dir = model_dir();
    FILES.iter().all(|(name, size)| std::fs::metadata(dir.join(name)).is_ok_and(|m| m.len() == *size))
}

/// Fetch the models that are missing (with `curl`; blocks, so call it from a worker thread).
pub fn fetch_models() -> Result<(), String> {
    let dir = model_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, size) in FILES {
        let path = dir.join(name);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() == size) {
            continue;
        }
        let part = dir.join(format!("{name}.part"));
        let status = std::process::Command::new("curl")
            .args(["-fsSL", "--retry", "2", "--connect-timeout", "20", "-o"])
            .arg(&part)
            .arg(format!("{BASE_URL}{name}"))
            .status()
            .map_err(|e| format!("curl: {e}"))?;
        let got = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if !status.success() || got != size {
            let _ = std::fs::remove_file(&part);
            return Err(format!("could not download {name} ({got} of {size} bytes)"));
        }
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The two models, ready to run on one of two engines.
pub struct Segmenter {
    engine: Engine,
}

enum Engine {
    /// onnxruntime (when its library is installed): about four times faster
    Ort { encoder: std::sync::Mutex<ort::session::Session>, decoder: std::sync::Mutex<ort::session::Session> },
    /// tract, built in: always there, slower
    Tract {
        encoder: Plan,
        decoder: Plan,
        /// the cores the models run on (tract alone uses one)
        pool: multithread::Executor,
    },
}

fn debug() -> bool {
    std::env::var_os("MEMRIUM_CUTOUT_DEBUG").is_some()
}

/// What the encoder made of one picture: the features the decoder reads for any point, and how the picture was fitted
/// into the encoder's square.
pub struct Embedding {
    /// 1 x 256 x 64 x 64
    features: Vec<f32>,
    /// the picture's colours on the decoder's grid (256 x 256 RGB, 0..1; the padding grey outside the picture)
    colours: Vec<[f32; 3]>,
    /// picture px -> encoder px
    scale: f32,
    w: usize,
    h: usize,
}

/// The mask of one object: logits on the decoder's grid (only the part under the point kept).
#[derive(Clone)]
pub struct Selection {
    low: Vec<f32>,
    pub score: f32,
}

impl Selection {
    /// The object's pixels out of `rgba` (the picture the embedding was made of), every `step`-th pixel each way: 1 for
    /// the full size, more for a quick, smaller copy (its `origin` is still in full-size picture px).
    pub fn render(&self, emb: &Embedding, rgba: &[u8], step: usize) -> Option<Cutout> {
        cutout(&self.low, emb, rgba, self.score, step.max(1))
    }

    /// Is the picture point `at` (picture px) on the object?
    pub fn contains(&self, emb: &Embedding, at: [f32; 2]) -> bool {
        let k = emb.scale * LOW as f32 / SIDE as f32;
        let (x, y) = ((at[0] * k) as usize, (at[1] * k) as usize);
        x < LOW && y < LOW && self.low[y * LOW + x] > 0.0
    }

    /// Add another object to this one (several lifted together): what is on either mask is on the result.
    pub fn union(&mut self, other: &Selection, emb: &Embedding) {
        for (a, b) in self.low.iter_mut().zip(&other.low) {
            *a = a.max(*b);
        }
        let region = [((emb.w as f32 * emb.scale / 4.0).ceil() as usize).clamp(1, LOW), ((emb.h as f32 * emb.scale / 4.0).ceil() as usize).clamp(1, LOW)];
        fill_holes(&mut self.low, region);
        self.score = self.score.min(other.score);
    }

    /// The object's box in picture px (x, y, w, h), from the grid: a little larger than the object.
    pub fn bounds(&self, emb: &Embedding) -> [f32; 4] {
        let k = emb.scale * LOW as f32 / SIDE as f32;
        let (mut x0, mut y0, mut x1, mut y1) = (LOW, LOW, 0, 0);
        for y in 0..LOW {
            for x in 0..LOW {
                if self.low[y * LOW + x] > 0.0 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        if x0 >= x1 {
            return [0.0; 4];
        }
        let (fx0, fy0) = (x0 as f32 / k, y0 as f32 / k);
        [fx0, fy0, (x1 as f32 / k).min(emb.w as f32) - fx0, (y1 as f32 / k).min(emb.h as f32) - fy0]
    }
}

/// An object lifted out: its pixels (straight RGBA, transparent around it) and where its top-left corner is in the picture.
pub struct Cutout {
    pub image: RgbaImage,
    pub origin: [u32; 2],
    /// how sure the model is that this is a whole object (0..1)
    pub score: f32,
}

fn err(e: impl std::fmt::Debug) -> String {
    format!("{e:?}")
}

/// The cores a run may take: most of them, leaving a couple for the program and the desktop.
fn threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).clamp(2, 8)
}

/// Load onnxruntime now, if it is there (call it first thing in `main`): loading it later, once a program has its
/// graphics and D-Bus threads running, was seen to hang for good waiting on the dynamic loader. About 30 ms.
pub fn preload() {
    let t = std::time::Instant::now();
    let ok = ort_ready();
    if debug() {
        eprintln!("cutout: onnxruntime preloaded ({ok}) in {:?}", t.elapsed());
    }
}

/// Is onnxruntime loaded (see `preload`)? The text recogniser (`crate::ocr`) runs on it too.
pub fn onnxruntime() -> bool {
    ort_ready()
}

/// Load onnxruntime once (`MEMRIUM_ONNXRUNTIME` or `ORT_DYLIB_PATH` names the library; the system's is tried too).
/// False when there is none, or `MEMRIUM_CUTOUT_ENGINE=tract` asks for the built-in engine.
fn ort_ready() -> bool {
    static READY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *READY.get_or_init(|| {
        if std::env::var("MEMRIUM_CUTOUT_ENGINE").is_ok_and(|e| e == "tract") {
            return false;
        }
        let mut paths: Vec<PathBuf> = ["MEMRIUM_ONNXRUNTIME", "ORT_DYLIB_PATH"].iter().filter_map(std::env::var_os).filter(|v| !v.is_empty()).map(PathBuf::from).collect();
        // a copy kept for Memrium (a Nix store link at ~/.local/share/memrium/onnxruntime, when the system's is too old)
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join(".local/share/memrium/onnxruntime/lib/libonnxruntime.so"));
        }
        paths.extend(["/run/current-system/sw/lib/libonnxruntime.so", "/usr/lib/libonnxruntime.so", "/usr/lib/x86_64-linux-gnu/libonnxruntime.so", "/usr/local/lib/libonnxruntime.so"].map(PathBuf::from));
        for p in paths.into_iter().filter(|p| p.exists()) {
            // onnxruntime 1.22 never comes back from being loaded (seen on NixOS: the whole program hangs before its
            // first line); a library whose store path names an older version than 1.23 is left alone
            if let Some((major, minor)) = version_in_path(&p) {
                if (major, minor) < (1, 23) {
                    eprintln!("cutout: onnxruntime {major}.{minor} at {} is too old (it hangs when loaded); 1.23 or newer is needed", p.display());
                    continue;
                }
            }
            if debug() {
                eprintln!("cutout: loading onnxruntime from {}", p.display());
            }
            if let Err(e) = loadable(&p) {
                eprintln!("cutout: onnxruntime at {} cannot be used ({e}); using tract", p.display());
                continue;
            }
            match ort::init_from(&p) {
                Ok(env) => {
                    env.with_name("memrium-cutout").commit();
                    if debug() {
                        eprintln!("cutout: onnxruntime from {}", p.display());
                    }
                    return true;
                }
                Err(e) => eprintln!("cutout: {}: {e}", p.display()),
            }
        }
        false
    })
}

/// Can `ort` load this library without trouble? Opened here first, because `ort` (2.0.0-rc.12) hangs for good when its
/// own loading fails: the error it builds asks for the API that is still being loaded (a `OnceLock` waiting on itself).
/// Seen with an onnxruntime from a newer nixpkgs than the program's: it needs a newer glibc (`GLIBC_2.43`), `dlopen`
/// refuses it, and the session never got past its first line. The library stays loaded on success (`ort` then gets the
/// same handle).
fn loadable(p: &Path) -> Result<(), String> {
    // SAFETY: loading onnxruntime runs its initialisers, as `ort::init_from` would do right after
    let lib = unsafe { libloading::Library::new(p) }.map_err(|e| e.to_string())?;
    // SAFETY: `OrtGetApiBase` takes nothing and returns a pointer to a static table; `GetVersionString` a static C string
    let version = unsafe {
        let get: libloading::Symbol<unsafe extern "C" fn() -> *const ort::sys::OrtApiBase> = lib.get(b"OrtGetApiBase").map_err(|_| "no OrtGetApiBase in it".to_string())?;
        let base = get();
        if base.is_null() {
            return Err("OrtGetApiBase gave nothing".into());
        }
        std::ffi::CStr::from_ptr(((*base).GetVersionString)()).to_string_lossy().into_owned()
    };
    let minor = version.split('.').nth(1).and_then(|m| m.parse::<u32>().ok()).unwrap_or(0);
    if minor < 22 {
        return Err(format!("version {version}, 1.22 or newer is needed"));
    }
    // (kept loaded: unloading and loading onnxruntime again is what was seen to hang before)
    std::mem::forget(lib);
    Ok(())
}

/// The onnxruntime version a library's (resolved) path names, as Nix store paths do: `...-onnxruntime-1.22.2/lib/...`.
fn version_in_path(p: &Path) -> Option<(u32, u32)> {
    let real = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = real.to_string_lossy();
    let at = s.find("onnxruntime-")? + "onnxruntime-".len();
    let mut nums = s[at..].split(|c: char| !c.is_ascii_digit()).filter(|t| !t.is_empty());
    Some((nums.next()?.parse().ok()?, nums.next()?.parse().ok()?))
}

impl Segmenter {
    pub fn load(dir: &Path) -> Result<Segmenter, String> {
        if debug() {
            eprintln!("cutout: loading the models from {}", dir.display());
        }
        if ort_ready() {
            match Self::load_ort(dir) {
                Ok(s) => return Ok(s),
                Err(e) => eprintln!("cutout: onnxruntime could not load the models ({e}); using tract"),
            }
        }
        Self::load_tract(dir)
    }

    /// Which engine runs the models ("onnxruntime" or "tract").
    pub fn engine(&self) -> &'static str {
        match self.engine {
            Engine::Ort { .. } => "onnxruntime",
            Engine::Tract { .. } => "tract",
        }
    }

    fn load_ort(dir: &Path) -> Result<Segmenter, String> {
        use ort::session::{builder::GraphOptimizationLevel, Session};
        let open = |name: &str| -> Result<Session, String> {
            Session::builder().map_err(err)?.with_optimization_level(GraphOptimizationLevel::Level3).map_err(err)?.with_intra_threads(threads()).map_err(err)?.commit_from_file(dir.join(name)).map_err(err)
        };
        Ok(Segmenter { engine: Engine::Ort { encoder: std::sync::Mutex::new(open(FILES[0].0)?), decoder: std::sync::Mutex::new(open(FILES[1].0)?) } })
    }

    fn load_tract(dir: &Path) -> Result<Segmenter, String> {
        // tract wants Pad's optional third input (the value to pad with) spelled out: zero, as the model means
        let mut src = std::fs::File::open(dir.join(FILES[0].0)).map_err(err)?;
        let mut proto = tract_onnx::onnx().proto_model_for_read(&mut src).map_err(err)?;
        if let Some(g) = proto.graph.as_mut() {
            let mut used = false;
            for n in g.node.iter_mut().filter(|n| n.op_type == "Pad" && n.input.get(2).is_none_or(|i| i.is_empty())) {
                n.input.truncate(2);
                n.input.push("memrium_pad_zero".into());
                used = true;
            }
            if used {
                g.initializer.push(tract_onnx::pb::TensorProto { name: "memrium_pad_zero".into(), dims: vec![], data_type: 1, float_data: vec![0.0], ..Default::default() });
            }
            split_softmax(g);
        }
        let encoder = tract_onnx::onnx()
            .model_for_proto_model(&proto)
            .map_err(err)?
            .with_input_fact(0, f32::fact([SIDE, SIDE, 3]).into())
            .map_err(err)?
            .into_optimized()
            .map_err(err)?
            .into_runnable()
            .map_err(err)?;
        // the "original" size never changes here (the encoder's square): baked in as a constant, the size arithmetic
        // folds away and tract sees plain shapes
        let mut src = std::fs::File::open(dir.join(FILES[1].0)).map_err(err)?;
        let mut proto = tract_onnx::onnx().proto_model_for_read(&mut src).map_err(err)?;
        if let Some(g) = proto.graph.as_mut() {
            let konst = |name: &str, dims: Vec<i64>, data: Vec<f32>| tract_onnx::pb::TensorProto { name: name.into(), dims, data_type: 1, float_data: data, ..Default::default() };
            g.input.retain(|i| i.name != "orig_im_size");
            g.initializer.push(konst("orig_im_size", vec![2], vec![SIDE as f32, SIDE as f32]));
        }
        let decoder = tract_onnx::onnx()
            .model_for_proto_model(&proto)
            .map_err(err)?
            .with_output_names(["iou_predictions", "low_res_masks"])
            .map_err(err)?
            .with_input_fact(0, f32::fact([1, 256, 64, 64]).into())
            .map_err(err)?
            .with_input_fact(1, f32::fact([1, POINTS, 2]).into())
            .map_err(err)?
            .with_input_fact(2, f32::fact([1, POINTS]).into())
            .map_err(err)?
            .with_input_fact(3, f32::fact([1, 1, LOW, LOW]).into())
            .map_err(err)?
            .with_input_fact(4, f32::fact([1]).into())
            .map_err(err)?
            .into_optimized()
            .map_err(err)?
            .into_runnable()
            .map_err(err)?;
        Ok(Segmenter { engine: Engine::Tract { encoder, decoder, pool: multithread::Executor::multithread_with_name(threads(), "cutout") } })
    }

    /// Run the encoder over a picture (`rgba` straight RGBA, `w * h * 4` bytes; transparent parts count as grey).
    pub fn embed(&self, rgba: &[u8], w: usize, h: usize) -> Result<Embedding, String> {
        let scale = SIDE as f32 / w.max(h) as f32;
        let (sw, sh) = (((w as f32 * scale).round() as usize).clamp(1, SIDE), ((h as f32 * scale).round() as usize).clamp(1, SIDE));
        let t = std::time::Instant::now();
        // (a fast box filter: the model sees a 1024 px picture, finer filtering buys nothing)
        let view = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(w as u32, h as u32, rgba).ok_or("bad picture size")?;
        let small = image::imageops::thumbnail(&view, sw as u32, sh as u32);
        // height x width x RGB, 0..255, the rest of the square in the mean colour (what the model's own padding means)
        let mut input: Vec<f32> = MEAN.iter().copied().cycle().take(SIDE * SIDE * 3).collect();
        for (x, y, p) in small.enumerate_pixels() {
            let a = p[3] as f32 / 255.0;
            let i = (y as usize * SIDE + x as usize) * 3;
            for c in 0..3 {
                input[i + c] = p[c] as f32 * a + MEAN[c] * (1.0 - a);
            }
        }
        let prep = t.elapsed();
        let input_copy = input.clone();
        let features = match &self.engine {
            Engine::Ort { encoder, .. } => {
                let x = ort::value::Tensor::from_array(([SIDE, SIDE, 3], input)).map_err(err)?;
                let mut s = encoder.lock().map_err(err)?;
                let out = s.run(ort::inputs!["input_image" => x]).map_err(err)?;
                out["image_embeddings"].try_extract_tensor::<f32>().map_err(err)?.1.to_vec()
            }
            Engine::Tract { encoder, pool, .. } => {
                let x = tract_ndarray::Array3::from_shape_vec((SIDE, SIDE, 3), input).map_err(err)?;
                if std::env::var_os("MEMRIUM_CUTOUT_PROFILE").is_some() {
                    profile(encoder, Tensor::from(x.clone()));
                }
                let out = multithread::multithread_tract_scope(pool.clone(), || encoder.run(tvec!(Tensor::from(x).into()))).map_err(err)?;
                out[0].to_array_view::<f32>().map_err(err)?.iter().copied().collect()
            }
        };
        if debug() {
            eprintln!("cutout: encoder ({}) prep {prep:?}, run {:?}", self.engine(), t.elapsed() - prep);
        }
        if features.len() != 256 * 64 * 64 {
            return Err(format!("the encoder gave {} values", features.len()));
        }
        // (the colours the masks are judged by: one per grid cell, the mean of its 4 x 4 encoder pixels)
        let mut colours = vec![[0f32; 3]; LOW * LOW];
        for gy in 0..LOW {
            for gx in 0..LOW {
                let mut c = [0f32; 3];
                for y in gy * 4..gy * 4 + 4 {
                    for x in gx * 4..gx * 4 + 4 {
                        let i = (y * SIDE + x) * 3;
                        for k in 0..3 {
                            c[k] += input_copy[i + k];
                        }
                    }
                }
                colours[gy * LOW + gx] = c.map(|v| v / (16.0 * 255.0));
            }
        }
        Ok(Embedding { features, colours, scale, w, h })
    }

    /// The object under the point `at` (picture px), lifted out of `rgba` (the same picture the embedding was made of).
    pub fn cut(&self, emb: &Embedding, rgba: &[u8], at: [f32; 2]) -> Result<Option<Cutout>, String> {
        self.cut_within(emb, rgba, at, 1.0)
    }

    /// As `cut`, taking no guess that covers more than `max_area` of the picture: on a screenshot a whole window is a
    /// guess too, and the click meant the thing in it.
    pub fn cut_within(&self, emb: &Embedding, rgba: &[u8], at: [f32; 2], max_area: f32) -> Result<Option<Cutout>, String> {
        let t = std::time::Instant::now();
        let sel = self.select_within(emb, at, max_area)?;
        let dec = t.elapsed();
        let c = sel.and_then(|s| s.render(emb, rgba, 1));
        if debug() {
            eprintln!("cutout: decoder {dec:?}, cut {:?}", t.elapsed() - dec);
        }
        Ok(c)
    }

    /// The object under `at` (picture px), as a mask: quick (two runs of the decoder); `Selection::render` makes its
    /// pixels. None when there is nothing there, or only the background.
    pub fn select(&self, emb: &Embedding, at: [f32; 2]) -> Result<Option<Selection>, String> {
        self.select_within(emb, at, 1.0)
    }

    /// As `select`, leaving out guesses that cover more than `max_area` of the picture.
    pub fn select_within(&self, emb: &Embedding, at: [f32; 2], max_area: f32) -> Result<Option<Selection>, String> {
        let click = [at[0] * emb.scale, at[1] * emb.scale];
        let seed = [(click[0] / 4.0) as usize, (click[1] / 4.0) as usize];
        let region = [((emb.w as f32 * emb.scale / 4.0).ceil() as usize).clamp(1, LOW), ((emb.h as f32 * emb.scale / 4.0).ceil() as usize).clamp(1, LOW)];
        // ---- the first look: one point, several guesses (a part, the object, the object and what it stands on)
        let (iou, masks) = self.decode(emb, &[(click, 1.0)], None)?;
        let n = (masks.len() / (LOW * LOW)).min(iou.len());
        let bg = background(emb, region);
        let mut guesses: Vec<(usize, Vec<f32>, f32, f32)> = Vec::new();
        for i in 0..n {
            let mut m = masks[i * LOW * LOW..(i + 1) * LOW * LOW].to_vec();
            keep_joined(&mut m, seed);
            let (area, border, sides) = coverage(&m, region);
            let plain = bg.map_or(0.0, |b| like_background(&m, emb, b));
            if debug() {
                eprintln!("cutout: guess {i}: iou {:.2}, area {:.2}, edge {:.2} on {sides} sides, background-coloured {:.2}", iou[i], area, border, plain);
            }
            // the background is not an object: a guess that runs along much of the picture's edge is left out, and so is
            // one made largely of the backdrop's colour (an object with a patch of the white wall beside it)
            // (and a guess the model itself doubts is no object)
            if area > 0.0 && area <= max_area && iou[i] >= MIN_IOU && border <= MAX_BORDER && sides < 2 && plain <= MAX_PLAIN {
                guesses.push((i, m, area, iou[i]));
            }
        }
        // of the guesses the model is fairly sure about (near its best, and not unsure), the biggest: the whole thing, not
        // one part of it (a click on the overalls lifts the bear)
        let best_iou = guesses.iter().map(|g| g.3).fold(f32::MIN, f32::max);
        let Some((first_i, first, first_area, first_iou)) = guesses.into_iter().filter(|g| g.3 >= (best_iou - 0.2).min(best_iou.max(0.7))).max_by(|a, b| a.2.total_cmp(&b.2)) else {
            return Ok(None);
        };
        // ---- the second look: the click and a few points well inside what was found, with that mask as a hint; the
        // answer no longer hangs on where exactly the click fell
        let mut points = vec![(click, 1.0)];
        for p in inner_points(&first, POINTS - 2) {
            points.push(([p[0] * 4.0 + 2.0, p[1] * 4.0 + 2.0], 1.0));
        }
        let raw = &masks[first_i * LOW * LOW..(first_i + 1) * LOW * LOW];
        let (iou2, masks2) = self.decode(emb, &points, Some(raw))?;
        let mut chosen = (first, first_iou);
        if !masks2.is_empty() && !iou2.is_empty() {
            // (with several points the decoder's single answer is the one to take)
            let mut m = masks2[..LOW * LOW].to_vec();
            keep_joined(&mut m, seed);
            let (area, border, sides) = coverage(&m, region);
            let plain = bg.map_or(0.0, |b| like_background(&m, emb, b));
            if debug() {
                eprintln!("cutout: refined: iou {:.2}, area {:.2}, edge {:.2}, background-coloured {:.2} ({} points)", iou2[0], area, border, plain, points.len());
            }
            if area <= max_area && border <= MAX_BORDER && sides < 2 && plain <= MAX_PLAIN && area > first_area * 0.6 && area < first_area * 1.8 && iou2[0] >= first_iou - 0.15 {
                chosen = (m, iou2[0]);
            }
        }
        let (mut low, score) = chosen;
        fill_holes(&mut low, region);
        Ok(Some(Selection { low, score: score.clamp(0.0, 1.0) }))
    }

    /// Run the decoder once on a fresh embedding: its first run is twice as slow as the next ones, and the first real
    /// one is a click someone waits on.
    pub fn warm(&self, emb: &Embedding) {
        let _ = self.decode(emb, &[([emb.w as f32 * emb.scale / 2.0, emb.h as f32 * emb.scale / 2.0], 1.0)], None);
    }

    /// One run of the decoder: `points` (encoder px, label 1 = on the object, 0 = not) and maybe an earlier mask's
    /// logits as a hint. Gives the guesses' confidences and their masks (n x 256 x 256 logits).
    fn decode(&self, emb: &Embedding, points: &[([f32; 2], f32)], hint: Option<&[f32]>) -> Result<(Vec<f32>, Vec<f32>), String> {
        let mut coords = Vec::with_capacity(POINTS * 2);
        let mut labels = Vec::with_capacity(POINTS);
        for (p, l) in points.iter().take(POINTS) {
            coords.extend_from_slice(p);
            labels.push(*l);
        }
        // the rest are padding (SAM's exported decoder wants at least one when no box is given)
        while labels.len() < POINTS {
            coords.extend_from_slice(&[0.0, 0.0]);
            labels.push(-1.0);
        }
        let mask_in = hint.map_or_else(|| vec![0.0f32; LOW * LOW], |h| h.to_vec());
        let has_mask = vec![if hint.is_some() { 1.0f32 } else { 0.0 }];
        match &self.engine {
            Engine::Ort { decoder, .. } => {
                use ort::value::Tensor as T;
                let mut s = decoder.lock().map_err(err)?;
                // (an "original" size of the low-resolution grid itself, so the full-size masks cost nothing)
                let out = s
                    .run(ort::inputs![
                        "image_embeddings" => T::from_array(([1usize, 256, 64, 64], emb.features.clone())).map_err(err)?,
                        "point_coords" => T::from_array(([1usize, POINTS, 2], coords)).map_err(err)?,
                        "point_labels" => T::from_array(([1usize, POINTS], labels)).map_err(err)?,
                        "mask_input" => T::from_array(([1usize, 1, LOW, LOW], mask_in)).map_err(err)?,
                        "has_mask_input" => T::from_array(([1usize], has_mask)).map_err(err)?,
                        "orig_im_size" => T::from_array(([2usize], vec![LOW as f32, LOW as f32])).map_err(err)?,
                    ])
                    .map_err(err)?;
                let iou = out["iou_predictions"].try_extract_tensor::<f32>().map_err(err)?.1.to_vec();
                let masks = out["low_res_masks"].try_extract_tensor::<f32>().map_err(err)?.1.to_vec();
                Ok((iou, masks))
            }
            Engine::Tract { decoder, pool, .. } => {
                let coords = tract_ndarray::Array3::from_shape_vec((1, POINTS, 2), coords).map_err(err)?;
                let labels = tract_ndarray::Array2::from_shape_vec((1, POINTS), labels).map_err(err)?;
                let feats = tract_ndarray::Array4::from_shape_vec((1, 256, 64, 64), emb.features.clone()).map_err(err)?;
                let mask_in = tract_ndarray::Array4::from_shape_vec((1, 1, LOW, LOW), mask_in).map_err(err)?;
                let has_mask = tract_ndarray::Array1::from_vec(has_mask);
                let inputs = tvec!(Tensor::from(feats).into(), Tensor::from(coords).into(), Tensor::from(labels).into(), Tensor::from(mask_in).into(), Tensor::from(has_mask).into());
                let out = multithread::multithread_tract_scope(pool.clone(), || decoder.run(inputs)).map_err(err)?;
                let iou = out[0].to_array_view::<f32>().map_err(err)?.iter().copied().collect();
                let masks = out[1].to_array_view::<f32>().map_err(err)?.iter().copied().collect();
                Ok((iou, masks))
            }
        }
    }
}

/// Guesses the model rates lower than this are not taken.
const MIN_IOU: f32 = 0.6;
/// A guess that covers more of the picture's edge than this is the background (or the object and the whole floor).
const MAX_BORDER: f32 = 0.22;
/// A side of the picture counts as touched when a guess lies along more than this share of it; an object may be cut
/// off by one side (a person by the bottom), one that runs into two is the backdrop (the sky, the table it lies on).
const SIDE_TOUCH: f32 = 0.06;
/// ... and so is one with more than this share of it in the colour of a plain backdrop.
const MAX_PLAIN: f32 = 0.12;
/// How near a colour has to be to the backdrop's to count as it (RGB distance, 0..1 per channel).
const NEAR_BACKDROP: f32 = 0.10;

/// The colour of a plain backdrop (a studio photo, a product on white), when the picture's edge is mostly of one colour.
/// None for a busy edge: then colour says nothing about what is background.
fn background(emb: &Embedding, region: [usize; 2]) -> Option<[f32; 3]> {
    let [rw, rh] = region;
    if rw < 4 || rh < 4 {
        return None;
    }
    let mut edge: Vec<[f32; 3]> = Vec::new();
    for x in 0..rw {
        edge.push(emb.colours[x]);
        edge.push(emb.colours[(rh - 1) * LOW + x]);
    }
    for y in 0..rh {
        edge.push(emb.colours[y * LOW]);
        edge.push(emb.colours[y * LOW + rw - 1]);
    }
    // the median colour, and how many edge cells are close to it
    let mut med = [0f32; 3];
    for k in 0..3 {
        let mut v: Vec<f32> = edge.iter().map(|c| c[k]).collect();
        v.sort_by(f32::total_cmp);
        med[k] = v[v.len() / 2];
    }
    let near = edge.iter().filter(|c| dist(**c, med) < NEAR_BACKDROP).count() as f32 / edge.len() as f32;
    (near > 0.7).then_some(med)
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The share of a mask's cells that are the backdrop's colour.
fn like_background(m: &[f32], emb: &Embedding, bg: [f32; 3]) -> f32 {
    let (mut n, mut plain) = (0usize, 0usize);
    for (i, v) in m.iter().enumerate() {
        if *v > 0.0 {
            n += 1;
            plain += (dist(emb.colours[i], bg) < NEAR_BACKDROP) as usize;
        }
    }
    plain as f32 / n.max(1) as f32
}

/// How much of the picture a mask covers, how much of the picture's edge, and on how many of its four sides it lies
/// along more than a little (both shares 0..1). `region` is the picture's part of the grid (the rest of the encoder's
/// square is padding).
fn coverage(m: &[f32], region: [usize; 2]) -> (f32, f32, usize) {
    let [rw, rh] = region;
    // (the decoder's masks often stop a cell short of the picture's edge: the outer two rows and columns count as it)
    let on = |x: usize, y: usize| (m[y * LOW + x] > 0.0) as usize;
    let near = |xs: [usize; 2], ys: [usize; 2]| on(xs[0], ys[0]).max(on(xs[1], ys[1]));
    let mut area = 0usize;
    for y in 0..rh {
        area += m[y * LOW..y * LOW + rw].iter().filter(|v| **v > 0.0).count();
    }
    let (r1, b1) = (rw.saturating_sub(2).max(0), rh.saturating_sub(2).max(0));
    let top: usize = (0..rw).map(|x| near([x, x], [0, 1.min(rh - 1)])).sum();
    let bottom: usize = (0..rw).map(|x| near([x, x], [rh - 1, b1])).sum();
    let left: usize = (0..rh).map(|y| near([0, 1.min(rw - 1)], [y, y])).sum();
    let right: usize = (0..rh).map(|y| near([rw - 1, r1], [y, y])).sum();
    let sides = [top as f32 / rw as f32, bottom as f32 / rw as f32, left as f32 / rh as f32, right as f32 / rh as f32].iter().filter(|f| **f > SIDE_TOUCH).count();
    let edge = (top + bottom + left + right) as f32 / (2 * (rw + rh)) as f32;
    (area as f32 / (rw * rh) as f32, edge, sides)
}

/// Up to `n` points (grid cells) well inside the mask and apart from each other: the deepest one, then the deepest
/// not near it, and so on.
fn inner_points(m: &[f32], n: usize) -> Vec<[f32; 2]> {
    // distance (in cells) to the nearest cell outside, two passes of a 3-4 chamfer
    let big = 1_000_000u32;
    let mut d: Vec<u32> = m.iter().map(|v| if *v > 0.0 { big } else { 0 }).collect();
    for y in 0..LOW {
        for x in 0..LOW {
            let i = y * LOW + x;
            if d[i] == 0 {
                continue;
            }
            let mut v = if x == 0 || y == 0 { 3 } else { d[i] };
            if x > 0 {
                v = v.min(d[i - 1] + 3);
            }
            if y > 0 {
                v = v.min(d[i - LOW] + 3);
                if x > 0 {
                    v = v.min(d[i - LOW - 1] + 4);
                }
                if x + 1 < LOW {
                    v = v.min(d[i - LOW + 1] + 4);
                }
            }
            d[i] = v;
        }
    }
    for y in (0..LOW).rev() {
        for x in (0..LOW).rev() {
            let i = y * LOW + x;
            if d[i] == 0 {
                continue;
            }
            let mut v = if x + 1 == LOW || y + 1 == LOW { 3 } else { d[i] };
            if x + 1 < LOW {
                v = v.min(d[i + 1] + 3);
            }
            if y + 1 < LOW {
                v = v.min(d[i + LOW] + 3);
                if x + 1 < LOW {
                    v = v.min(d[i + LOW + 1] + 4);
                }
                if x > 0 {
                    v = v.min(d[i + LOW - 1] + 4);
                }
            }
            d[i] = v;
        }
    }
    let deepest = d.iter().copied().max().unwrap_or(0);
    let mut out: Vec<[f32; 2]> = Vec::new();
    // (points too close to the edge say little: at least two cells in, and a fifth as deep as the deepest)
    let floor = (deepest / 5).max(6);
    while out.len() < n {
        let Some((i, &v)) = d.iter().enumerate().max_by_key(|(_, v)| **v) else { break };
        if v < floor {
            break;
        }
        let (x, y) = ((i % LOW) as f32, (i / LOW) as f32);
        out.push([x, y]);
        // nothing else near this one: a disc as wide as the deepest distance
        let r = (deepest as f32 / 3.0).max(4.0);
        let (x0, x1) = ((x - r).max(0.0) as usize, ((x + r) as usize).min(LOW - 1));
        let (y0, y1) = ((y - r).max(0.0) as usize, ((y + r) as usize).min(LOW - 1));
        for yy in y0..=y1 {
            for xx in x0..=x1 {
                if (xx as f32 - x).hypot(yy as f32 - y) <= r {
                    d[yy * LOW + xx] = 0;
                }
            }
        }
    }
    out
}

/// Small holes inside the object (a gap between an arm and the body stays; a speck on a sleeve is filled): cells that
/// the picture's edge cannot reach without crossing the mask, in pockets of under 1.5 % of the object.
fn fill_holes(m: &mut [f32], region: [usize; 2]) {
    let [rw, rh] = region;
    let inside = |x: usize, y: usize| x < rw && y < rh;
    let mut seen = vec![false; LOW * LOW];
    let area = m.iter().filter(|v| **v > 0.0).count();
    let limit = (area as f32 * 0.015) as usize;
    for y in 0..rh {
        for x in 0..rw {
            let i = y * LOW + x;
            if seen[i] || m[i] > 0.0 {
                continue;
            }
            // one pocket of outside cells
            let mut pocket = vec![i];
            let mut stack = vec![i];
            seen[i] = true;
            let mut open = false;
            while let Some(j) = stack.pop() {
                let (cx, cy) = (j % LOW, j / LOW);
                if cx == 0 || cy == 0 || cx + 1 >= rw || cy + 1 >= rh {
                    open = true;
                }
                for (nx, ny) in [(cx.wrapping_sub(1), cy), (cx + 1, cy), (cx, cy.wrapping_sub(1)), (cx, cy + 1)] {
                    if inside(nx, ny) {
                        let k = ny * LOW + nx;
                        if !seen[k] && m[k] <= 0.0 {
                            seen[k] = true;
                            stack.push(k);
                            pocket.push(k);
                        }
                    }
                }
            }
            if !open && pocket.len() <= limit {
                for j in pocket {
                    m[j] = 2.0;
                }
            }
        }
    }
}

/// Time each kind of node of tract's encoder once (`MEMRIUM_CUTOUT_PROFILE`), slowest first.
fn profile(encoder: &Plan, input: Tensor) {
    use std::collections::HashMap;
    let mut times: HashMap<String, (std::time::Duration, usize)> = HashMap::new();
    let Ok(mut state) = SimpleState::new(encoder) else { return };
    let r = state.run_plan_with_eval(tvec!(input.into()), |s, st, node, inp| {
        let t = std::time::Instant::now();
        let r = tract_onnx::tract_core::plan::eval(s, st, node, inp);
        let e = times.entry(node.op().name().to_string()).or_default();
        e.0 += t.elapsed();
        e.1 += 1;
        r
    });
    let mut v: Vec<_> = times.into_iter().collect();
    v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    eprintln!("cutout profile ({}):", if r.is_ok() { "ok" } else { "failed" });
    for (op, (d, n)) in v.iter().take(14) {
        eprintln!("  {op:<28} {n:>4}x {d:?}");
    }
}

/// tract's own Softmax is very slow on the attention maps of this encoder (11 of 17 seconds): each one is written out
/// as the plain steps it stands for, `exp(x - max) / sum(exp(x - max))`, which tract runs as fast vector code.
fn split_softmax(g: &mut tract_onnx::pb::GraphProto) {
    use tract_onnx::pb::{AttributeProto, NodeProto, TensorProto};
    let ints = |name: &str, v: Vec<i64>| AttributeProto { name: name.into(), r#type: 7, ints: v, ..Default::default() };
    let int = |name: &str, v: i64| AttributeProto { name: name.into(), r#type: 2, i: v, ..Default::default() };
    let mut nodes = Vec::with_capacity(g.node.len() + 64);
    let mut extra = Vec::new();
    for (k, n) in std::mem::take(&mut g.node).into_iter().enumerate() {
        if n.op_type != "Softmax" || n.input.len() != 1 || n.output.len() != 1 {
            nodes.push(n);
            continue;
        }
        let axis = n.attribute.iter().find(|a| a.name == "axis").map_or(-1, |a| a.i);
        let (x, y) = (n.input[0].clone(), n.output[0].clone());
        let t = |s: &str| format!("memrium_softmax{k}_{s}");
        let node = |op: &str, input: Vec<String>, out: String, attribute: Vec<AttributeProto>| NodeProto { op_type: op.into(), name: format!("{out}_node"), input, output: vec![out], attribute, ..Default::default() };
        nodes.push(node("ReduceMax", vec![x.clone()], t("max"), vec![ints("axes", vec![axis]), int("keepdims", 1)]));
        nodes.push(node("Sub", vec![x, t("max")], t("shift"), vec![]));
        nodes.push(node("Exp", vec![t("shift")], t("exp"), vec![]));
        nodes.push(node("ReduceSum", vec![t("exp"), t("axes")], t("sum"), vec![int("keepdims", 1)]));
        nodes.push(node("Div", vec![t("exp"), t("sum")], y, vec![]));
        extra.push(TensorProto { name: t("axes"), dims: vec![1], data_type: 7, int64_data: vec![axis], ..Default::default() });
    }
    g.node = nodes;
    g.initializer.extend(extra);
}

/// Keep only the part of the mask that touches `seed` (or, when the point itself fell just outside it, the nearest
/// part): stray specks elsewhere in the picture are not part of the object.
fn keep_joined(low: &mut [f32], seed: [usize; 2]) {
    let (sx, sy) = (seed[0].min(LOW - 1), seed[1].min(LOW - 1));
    let mut start = None;
    'find: for r in 0..12i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (sx as i32 + dx, sy as i32 + dy);
                if (0..LOW as i32).contains(&x) && (0..LOW as i32).contains(&y) && low[y as usize * LOW + x as usize] > 0.0 {
                    start = Some(y as usize * LOW + x as usize);
                    break 'find;
                }
            }
        }
    }
    let Some(start) = start else {
        low.iter_mut().for_each(|v| *v = -20.0);
        return;
    };
    let mut seen = vec![false; LOW * LOW];
    let mut stack = vec![start];
    seen[start] = true;
    while let Some(i) = stack.pop() {
        let (x, y) = (i % LOW, i / LOW);
        for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
            if nx < LOW && ny < LOW {
                let j = ny * LOW + nx;
                if !seen[j] && low[j] > 0.0 {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    // cells next to the kept part keep their (negative) value so the edge stays soft; the rest is pushed far out
    for i in 0..LOW * LOW {
        if !seen[i] && low[i] > 0.0 {
            low[i] = -20.0;
        }
    }
}

/// Cut the masked object out of the picture: a tight box around it, every `step`-th pixel. The mask's grid is coarse
/// (a cell is several pixels of a big picture), so its edge is only a guess at where the object ends: in a band along
/// that edge the alpha is settled by a guided filter over the picture's own brightness, which makes it follow the real
/// border (fur, a sleeve) instead of the grid. Away from the band it is simply in or out.
fn cutout(low: &[f32], emb: &Embedding, rgba: &[u8], score: f32, step: usize) -> Option<Cutout> {
    // the box, from the grid (a cell spans 1 / k picture px)
    let k = emb.scale * LOW as f32 / SIDE as f32;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..LOW {
        for x in 0..LOW {
            if low[y * LOW + x] > 0.0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 > x1 {
        return None;
    }
    // (a cell of margin all round: the real edge may lie a little outside the grid's)
    let to_pic = |c: f32, len: usize| ((c / k).floor().max(0.0) as usize).min(len);
    let (bx0, by0) = (to_pic(x0 as f32 - 1.0, emb.w), to_pic(y0 as f32 - 1.0, emb.h));
    let (bx1, by1) = (to_pic(x1 as f32 + 2.0, emb.w), to_pic(y1 as f32 + 2.0, emb.h));
    if bx1 <= bx0 + 1 || by1 <= by0 + 1 {
        return None;
    }
    let (cw, ch) = ((bx1 - bx0).div_ceil(step), (by1 - by0).div_ceil(step));
    let n = cw * ch;
    // grid cells per output pixel, and output pixels per cell
    let kk = k * step as f32;
    let cell = 1.0 / kk;
    // ---- the coarse alpha from the grid, and where it is undecided
    let mut p = vec![0f32; n];
    let mut band = vec![false; n];
    let mut guide = vec![0f32; n];
    let src_px = |x: usize, y: usize| ((by0 + y * step).min(emb.h - 1), (bx0 + x * step).min(emb.w - 1));
    for y in 0..ch {
        for x in 0..cw {
            let (py, px) = src_px(x, y);
            let fx = (px as f32 + 0.5) * k - 0.5;
            let fy = (py as f32 + 0.5) * k - 0.5;
            let gx0 = fx.floor().clamp(0.0, (LOW - 1) as f32) as usize;
            let gy0 = fy.floor().clamp(0.0, (LOW - 1) as f32) as usize;
            let (gx1, gy1) = ((gx0 + 1).min(LOW - 1), (gy0 + 1).min(LOW - 1));
            let (tx, ty) = ((fx - gx0 as f32).clamp(0.0, 1.0), (fy - gy0 as f32).clamp(0.0, 1.0));
            let g = |x: usize, y: usize| low[y * LOW + x].clamp(-8.0, 8.0);
            let l = (g(gx0, gy0) * (1.0 - tx) + g(gx1, gy0) * tx) * (1.0 - ty) + (g(gx0, gy1) * (1.0 - tx) + g(gx1, gy1) * tx) * ty;
            let i = y * cw + x;
            p[i] = 1.0 / (1.0 + (-1.5 * l).exp());
            // undecided: within about a cell of the grid's edge
            band[i] = l.abs() < 3.0;
            let s = &rgba[(py * emb.w + px) * 4..(py * emb.w + px) * 4 + 3];
            guide[i] = (0.299 * s[0] as f32 + 0.587 * s[1] as f32 + 0.114 * s[2] as f32) / 255.0;
        }
    }
    // ---- the guided filter: the coarse alpha, made to follow the picture's edges (He, Sun, Tang 2010)
    let r = (cell * 1.3).round().clamp(2.0, 28.0) as usize;
    let q = guided(&guide, &p, cw, ch, r, 0.003);
    let mut out = vec![0u8; n * 4];
    for y in 0..ch {
        for x in 0..cw {
            let i = y * cw + x;
            let a = if band[i] {
                // a firmer edge than the filter's own ramp
                let t = ((q[i] - 0.5) * 1.8 + 0.5).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            } else if p[i] > 0.5 {
                1.0
            } else {
                0.0
            };
            if a < 0.02 {
                continue;
            }
            let (py, px) = src_px(x, y);
            let s = &rgba[(py * emb.w + px) * 4..(py * emb.w + px) * 4 + 4];
            out[i * 4..i * 4 + 3].copy_from_slice(&s[..3]);
            out[i * 4 + 3] = (s[3] as f32 * a).round() as u8;
        }
    }
    // trim to what is really there
    let (mut minx, mut miny, mut maxx, mut maxy) = (cw, ch, 0, 0);
    for y in 0..ch {
        let row = &out[y * cw * 4..(y + 1) * cw * 4];
        if let Some(f) = (0..cw).find(|&x| row[x * 4 + 3] != 0) {
            let last = (0..cw).rev().find(|&x| row[x * 4 + 3] != 0).unwrap_or(f);
            minx = minx.min(f);
            maxx = maxx.max(last);
            miny = miny.min(y);
            maxy = maxy.max(y);
        }
    }
    if maxx < minx || maxy < miny {
        return None;
    }
    let full = RgbaImage::from_raw(cw as u32, ch as u32, out)?;
    let image = image::imageops::crop_imm(&full, minx as u32, miny as u32, (maxx - minx + 1) as u32, (maxy - miny + 1) as u32).to_image();
    Some(Cutout { image, origin: [(bx0 + minx * step) as u32, (by0 + miny * step) as u32], score })
}

/// Mean of `v` over a (2r+1)-square window round each pixel (cut at the picture's edges), from a summed-area table.
fn box_mean(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut sat = vec![0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0f64;
        for x in 0..w {
            row += v[y * w + x] as f64;
            sat[(y + 1) * (w + 1) + x + 1] = sat[y * (w + 1) + x + 1] + row;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        let (ya, yb) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (xa, xb) = (x.saturating_sub(r), (x + r + 1).min(w));
            let s = sat[yb * (w + 1) + xb] - sat[ya * (w + 1) + xb] - sat[yb * (w + 1) + xa] + sat[ya * (w + 1) + xa];
            out[y * w + x] = (s / ((yb - ya) * (xb - xa)) as f64) as f32;
        }
    }
    out
}

/// The guided filter: `p` smoothed so that its edges follow those of the guide `i` (both w x h, 0..1).
fn guided(i: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let ip: Vec<f32> = i.iter().zip(p).map(|(a, b)| a * b).collect();
    let ii: Vec<f32> = i.iter().map(|a| a * a).collect();
    let (mi, mp, mip, mii) = (box_mean(i, w, h, r), box_mean(p, w, h, r), box_mean(&ip, w, h, r), box_mean(&ii, w, h, r));
    let mut a = vec![0f32; w * h];
    let mut b = vec![0f32; w * h];
    for k in 0..w * h {
        let var = mii[k] - mi[k] * mi[k];
        let cov = mip[k] - mi[k] * mp[k];
        a[k] = cov / (var + eps);
        b[k] = mp[k] - a[k] * mi[k];
    }
    let (ma, mb) = (box_mean(&a, w, h, r), box_mean(&b, w, h, r));
    (0..w * h).map(|k| ma[k] * i[k] + mb[k]).collect()
}

/// Straight RGBA from the premultiplied BGRA pictures are kept in.
pub fn rgba_from_bgra_premul(bgra: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bgra.len());
    for p in bgra.chunks_exact(4) {
        let a = p[3];
        let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
        out.extend_from_slice(&[un(p[2]), un(p[1]), un(p[0]), a]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_with(blobs: &[[usize; 4]]) -> Vec<f32> {
        let mut g = vec![-5.0f32; LOW * LOW];
        for b in blobs {
            for y in b[1]..b[3] {
                for x in b[0]..b[2] {
                    g[y * LOW + x] = 4.0;
                }
            }
        }
        g
    }

    #[test]
    fn only_the_part_under_the_point_is_kept() {
        let mut g = grid_with(&[[10, 10, 40, 40], [100, 100, 140, 140]]);
        keep_joined(&mut g, [20, 20]);
        assert!(g[20 * LOW + 20] > 0.0);
        assert!(g[120 * LOW + 120] < -10.0, "the other blob is dropped");
        // the soft edge round the kept part is left as it was
        assert_eq!(g[5 * LOW + 5], -5.0);
    }

    #[test]
    fn a_point_just_beside_the_object_still_finds_it() {
        let mut g = grid_with(&[[50, 50, 80, 80]]);
        keep_joined(&mut g, [47, 60]);
        assert!(g[60 * LOW + 60] > 0.0);
    }

    #[test]
    fn a_point_far_from_anything_lifts_nothing() {
        let mut g = grid_with(&[[50, 50, 80, 80]]);
        keep_joined(&mut g, [200, 200]);
        assert!(g.iter().all(|v| *v <= 0.0));
    }
}
