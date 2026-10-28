//! A PipeWire video source node that shows the pictures it is given. The screen-sharing frontend connects programs to
//! its node id.

use pipewire as pw;
use pw::{properties::properties, spa};
use spa::pod::{Object, Pod, Property, PropertyFlags, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

/// One picture: BGRx, `w * 4` bytes per row.
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub bgrx: Vec<u8>,
}

enum Cmd {
    Frame(Frame),
    Quit,
}

pub struct PwStream {
    tx: pw::channel::Sender<Cmd>,
    pub node_id: u32,
    thread: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

#[derive(Default)]
struct Shared {
    latest: Option<Frame>,
    /// the size the stream was last negotiated with
    size: (u32, u32),
}

fn format_pod(w: u32, h: u32) -> Vec<u8> {
    let prop = |key: u32, value: Value| Property { key, flags: PropertyFlags::empty(), value };
    let obj = Value::Object(Object {
        type_: spa::sys::SPA_TYPE_OBJECT_Format,
        id: spa::sys::SPA_PARAM_EnumFormat,
        properties: vec![
            prop(spa::sys::SPA_FORMAT_mediaType, Value::Id(spa::utils::Id(spa::sys::SPA_MEDIA_TYPE_video))),
            prop(spa::sys::SPA_FORMAT_mediaSubtype, Value::Id(spa::utils::Id(spa::sys::SPA_MEDIA_SUBTYPE_raw))),
            prop(spa::sys::SPA_FORMAT_VIDEO_format, Value::Id(spa::utils::Id(spa::sys::SPA_VIDEO_FORMAT_BGRx))),
            prop(spa::sys::SPA_FORMAT_VIDEO_size, Value::Rectangle(spa::utils::Rectangle { width: w, height: h })),
            // 0 / 1: the pace is the picture's own
            prop(spa::sys::SPA_FORMAT_VIDEO_framerate, Value::Fraction(spa::utils::Fraction { num: 0, denom: 1 })),
        ],
    });
    spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &obj).expect("a format pod").0.into_inner()
}

fn buffers_pod(w: u32, h: u32) -> Vec<u8> {
    let int = |key: u32, v: i32| Property { key, flags: PropertyFlags::empty(), value: Value::Int(v) };
    let obj = Value::Object(Object {
        type_: spa::sys::SPA_TYPE_OBJECT_ParamBuffers,
        id: spa::sys::SPA_PARAM_Buffers,
        properties: vec![
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_buffers,
                flags: PropertyFlags::empty(),
                value: Value::Choice(spa::pod::ChoiceValue::Int(spa::utils::Choice(spa::utils::ChoiceFlags::empty(), spa::utils::ChoiceEnum::Range { default: 4, min: 2, max: 8 }))),
            },
            int(spa::sys::SPA_PARAM_BUFFERS_blocks, 1),
            int(spa::sys::SPA_PARAM_BUFFERS_size, (w * h * 4) as i32),
            int(spa::sys::SPA_PARAM_BUFFERS_stride, (w * 4) as i32),
            // a set of kinds to choose from: a plain Int must equal the consumer's (OBS asks for MemFd alone, and 6 != 4
            // fails the link with "error alloc buffers")
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_dataType,
                flags: PropertyFlags::empty(),
                value: Value::Choice(spa::pod::ChoiceValue::Int(spa::utils::Choice(
                    spa::utils::ChoiceFlags::empty(),
                    spa::utils::ChoiceEnum::Flags { default: 1 << spa::sys::SPA_DATA_MemFd | 1 << spa::sys::SPA_DATA_MemPtr, flags: vec![1 << spa::sys::SPA_DATA_MemFd, 1 << spa::sys::SPA_DATA_MemPtr] },
                ))),
            },
        ],
    });
    spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &obj).expect("a buffers pod").0.into_inner()
}

impl PwStream {
    /// Make the node, `w` x `h` to begin with. Waits until the daemon has it, so the node id is known.
    pub fn start(w: u32, h: u32, name: &str) -> Result<PwStream, String> {
        let (tx, rx) = pw::channel::channel::<Cmd>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
        let name = name.to_string();
        let thread = std::thread::spawn(move || {
            if let Err(e) = run(w, h, &name, rx, &ready_tx) {
                let _ = ready_tx.send(Err(e.to_string()));
            }
        });
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(node_id)) => Ok(PwStream { tx, node_id, thread: std::sync::Mutex::new(Some(thread)) }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("PipeWire did not answer".into()),
        }
    }

    pub fn push(&self, frame: Frame) {
        let _ = self.tx.send(Cmd::Frame(frame));
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Quit);
        if let Some(t) = self.thread.lock().ok().and_then(|mut t| t.take()) {
            let _ = t.join();
        }
    }
}

fn run(w: u32, h: u32, name: &str, rx: pw::channel::Receiver<Cmd>, ready: &mpsc::Sender<Result<u32, String>>) -> Result<(), pw::Error> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let shared = Rc::new(RefCell::new(Shared { latest: None, size: (w, h) }));

    let stream = pw::stream::StreamBox::new(
        &core,
        name,
        properties! {
            *pw::keys::MEDIA_CLASS => "Video/Source",
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
            *pw::keys::NODE_NAME => "memrium-screencast",
        },
    )?;

    let ready_state = ready.clone();
    let listener = stream
        .add_local_listener_with_user_data(shared.clone())
        .state_changed(move |stream, _, _, new| {
            if let pw::stream::StreamState::Paused = new {
                let _ = ready_state.send(Ok(stream.node_id()));
            }
            if let pw::stream::StreamState::Error(e) = new {
                let _ = ready_state.send(Err(e));
            }
        })
        .param_changed(|stream, shared, id, param| {
            let Some(param) = param else { return };
            if id != spa::sys::SPA_PARAM_Format {
                return;
            }
            let mut info = spa::param::video::VideoInfoRaw::new();
            if info.parse(param).is_err() {
                return;
            }
            let size = info.size();
            shared.borrow_mut().size = (size.width, size.height);
            let bytes = buffers_pod(size.width, size.height);
            if let Some(pod) = Pod::from_bytes(&bytes) {
                let _ = stream.update_params(&mut [pod]);
            }
        })
        .process(|stream, shared| {
            let sh = shared.borrow();
            let Some(frame) = sh.latest.as_ref() else { return };
            if (frame.w, frame.h) != sh.size {
                // the shared window changed its size: offer the new one
                let bytes = format_pod(frame.w, frame.h);
                if let Some(pod) = Pod::from_bytes(&bytes) {
                    let _ = stream.update_params(&mut [pod]);
                }
                return;
            }
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let Some(data) = buffer.datas_mut().first_mut() else { return };
            let stride = frame.w as usize * 4;
            let copied = match data.data() {
                Some(dst) if dst.len() >= frame.bgrx.len() => {
                    dst[..frame.bgrx.len()].copy_from_slice(&frame.bgrx);
                    frame.bgrx.len()
                }
                _ => 0,
            };
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = stride as i32;
            *chunk.size_mut() = copied as u32;
        })
        .register()?;

    let bytes = format_pod(w, h);
    let mut params = [Pod::from_bytes(&bytes).expect("a format pod")];
    stream.connect(spa::utils::Direction::Output, None, pw::stream::StreamFlags::MAP_BUFFERS, &mut params)?;

    let quit = mainloop.clone();
    let latest = shared.clone();
    let _recv = rx.attach(mainloop.loop_(), move |cmd| match cmd {
        Cmd::Frame(f) => {
            latest.borrow_mut().latest = Some(f);
        }
        Cmd::Quit => quit.quit(),
    });
    mainloop.run();
    drop(listener);
    Ok(())
}
