//! `orochi-servo`: Servo as the engine behind the window's browser tab. It speaks the
//! protocol of `docs/servo-browser-design.md` §3.2 on stdin and stdout and logs on stderr.
mod input;
mod out;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use servo::{AllowOrDenyRequest, CreateNewWebViewRequest, NavigationRequest, PermissionRequest};
use servo::{
    ConsoleLogLevel, Cursor, EmbedderControl, EventLoopWaker, LoadStatus, Opts, RenderingContext,
    RgbaImage, ServoBuilder, SimpleDialog, SoftwareRenderingContext, WebView, WebViewBuilder,
    WebViewDelegate,
};

const FRAME_INTERVAL: Duration = Duration::from_millis(50);

enum Message {
    Command(Value),
    Wake,
    Eof,
}

#[derive(Clone)]
struct Waker(mpsc::Sender<Message>);
impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {
        let _ = self.0.send(Message::Wake);
    }
}

struct Args {
    profile: Option<PathBuf>,
    width: u32,
    height: u32,
    quality: u8,
    exec_command: bool,
    temporary: bool,
    frames: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        profile: None,
        width: 1024,
        height: 768,
        quality: 70,
        exec_command: true,
        temporary: false,
        frames: true,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = it.next().unwrap_or_default();
        match flag.as_str() {
            "--profile" => args.profile = Some(PathBuf::from(value)),
            "--width" => args.width = value.parse().unwrap_or(args.width),
            "--height" => args.height = value.parse().unwrap_or(args.height),
            "--quality" => args.quality = value.parse().unwrap_or(args.quality).clamp(1, 100),
            "--exec-command" => args.exec_command = value == "1",
            "--frames" => args.frames = value != "0",
            "--temporary-storage" => args.temporary = value == "1",
            other => {
                eprintln!("orochi-servo: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    args
}

fn js_json(value: servo::JSValue) -> Value {
    use servo::JSValue as J;
    match value {
        J::Undefined | J::Null => Value::Null,
        J::Boolean(b) => json!(b),
        J::Number(n) => json!(n),
        J::String(s) | J::Element(s) | J::ShadowRoot(s) | J::Frame(s) | J::Window(s) => json!(s),
        J::Array(items) => Value::Array(items.into_iter().map(js_json).collect()),
        J::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, js_json(v))).collect()),
    }
}

fn cursor_name(cursor: Cursor) -> &'static str {
    match cursor {
        Cursor::None => "none",
        Cursor::Default => "default",
        Cursor::Pointer => "pointer",
        Cursor::ContextMenu => "context-menu",
        Cursor::Help => "help",
        Cursor::Progress => "progress",
        Cursor::Wait => "wait",
        Cursor::Cell => "cell",
        Cursor::Crosshair => "crosshair",
        Cursor::Text => "text",
        Cursor::VerticalText => "vertical-text",
        Cursor::Alias => "alias",
        Cursor::Copy => "copy",
        Cursor::Move => "move",
        Cursor::NoDrop => "no-drop",
        Cursor::NotAllowed => "not-allowed",
        Cursor::Grab => "grab",
        Cursor::Grabbing => "grabbing",
        Cursor::EResize => "e-resize",
        Cursor::NResize => "n-resize",
        Cursor::NeResize => "ne-resize",
        Cursor::NwResize => "nw-resize",
        Cursor::SResize => "s-resize",
        Cursor::SeResize => "se-resize",
        Cursor::SwResize => "sw-resize",
        Cursor::WResize => "w-resize",
        Cursor::EwResize => "ew-resize",
        Cursor::NsResize => "ns-resize",
        Cursor::NeswResize => "nesw-resize",
        Cursor::NwseResize => "nwse-resize",
        Cursor::ColResize => "col-resize",
        Cursor::RowResize => "row-resize",
        Cursor::AllScroll => "all-scroll",
        Cursor::ZoomIn => "zoom-in",
        Cursor::ZoomOut => "zoom-out",
    }
}

/// Only web addresses and the empty page are ever shown, whoever asks.
fn allowed(url: &str) -> bool {
    url == "about:blank"
        || url::Url::parse(url)
            .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
}

#[derive(Default)]
struct Delegate {
    ready: Cell<bool>,
    dirty: Cell<bool>,
    animating: Cell<bool>,
    dialog: RefCell<Option<SimpleDialog>>,
}

impl WebViewDelegate for Delegate {
    fn notify_new_frame_ready(&self, webview: WebView) {
        webview.paint();
        self.dirty.set(true);
    }
    fn notify_animating_changed(&self, _: WebView, animating: bool) {
        self.animating.set(animating);
    }
    fn notify_url_changed(&self, _: WebView, url: url::Url) {
        out::event(json!({"event": "url", "url": url.as_str()}));
    }
    fn notify_page_title_changed(&self, _: WebView, title: Option<String>) {
        out::event(json!({"event": "title", "title": title.unwrap_or_default()}));
    }
    fn notify_history_changed(&self, _: WebView, entries: Vec<url::Url>, current: usize) {
        out::event(json!({"event": "history", "back": current > 0,
                          "forward": current + 1 < entries.len()}));
    }
    fn notify_load_status_changed(&self, _: WebView, status: LoadStatus) {
        let state = match status {
            LoadStatus::Started => "started",
            LoadStatus::Complete | LoadStatus::Stopped => "complete",
            _ => return,
        };
        out::event(json!({"event": "loading", "state": state}));
        // A navigation asked for while the first blank page is still loading is lost.
        if state == "complete" && !self.ready.replace(true) {
            out::event(json!({"event": "ready"}));
        }
    }
    fn notify_cursor_changed(&self, _: WebView, cursor: Cursor) {
        out::event(json!({"event": "cursor", "cursor": cursor_name(cursor)}));
    }
    fn notify_crashed(&self, _: WebView, reason: String, _: Option<String>) {
        eprintln!("orochi-servo: content crashed: {reason}");
        out::event(json!({"event": "crashed"}));
    }
    fn request_navigation(&self, _: WebView, request: NavigationRequest) {
        if allowed(request.url.as_str()) {
            request.allow();
        } else {
            out::event(json!({"event": "refused", "url": request.url.as_str()}));
            request.deny();
        }
    }
    fn request_create_new(&self, _: WebView, _: CreateNewWebViewRequest) {
        // The request does not carry the address; dropping it declines the window.
        out::event(json!({"event": "popup", "url": ""}));
    }
    fn request_permission(&self, _: WebView, request: PermissionRequest) {
        request.deny();
    }
    fn request_unload(&self, _: WebView, request: AllowOrDenyRequest) {
        request.allow();
    }
    fn show_console_message(&self, _: WebView, level: ConsoleLogLevel, message: String) {
        let level = match level {
            ConsoleLogLevel::Warn => "warn",
            ConsoleLogLevel::Error => "error",
            ConsoleLogLevel::Debug => "debug",
            ConsoleLogLevel::Info => "info",
            _ => "log",
        };
        let text: String = message.chars().take(2000).collect();
        out::event(json!({"event": "console", "level": level, "text": text}));
    }
    fn show_embedder_control(&self, _: WebView, control: EmbedderControl) {
        match control {
            EmbedderControl::SimpleDialog(dialog) => {
                let (kind, default) = match &dialog {
                    SimpleDialog::Alert(_) => ("alert", String::new()),
                    SimpleDialog::Confirm(_) => ("confirm", String::new()),
                    SimpleDialog::Prompt(p) => ("prompt", p.current_value().to_owned()),
                };
                out::event(json!({"event": "dialog", "kind": kind,
                                  "message": dialog.message(), "default": default}));
                *self.dialog.borrow_mut() = Some(dialog);
            }
            // Not drawn yet: dropping a control answers it with its default.
            EmbedderControl::SelectElement(_) => eprintln!("orochi-servo: <select> not drawn"),
            EmbedderControl::ColorPicker(_) => eprintln!("orochi-servo: color picker not drawn"),
            EmbedderControl::FilePicker(_) => eprintln!("orochi-servo: file picker not drawn"),
            EmbedderControl::ContextMenu(_) => {}
            EmbedderControl::InputMethod(_) => {}
        }
    }
    fn hide_embedder_control(&self, _: WebView, _: servo::EmbedderControlId) {
        if self.dialog.borrow_mut().take().is_some() {
            out::event(json!({"event": "dialog_closed"}));
        }
    }
}

struct Frames {
    sender: mpsc::SyncSender<(u64, RgbaImage)>,
    seq: u64,
    last: Instant,
}

fn frame_worker(quality: u8) -> mpsc::SyncSender<(u64, RgbaImage)> {
    let (sender, receiver) = mpsc::sync_channel::<(u64, RgbaImage)>(1);
    std::thread::spawn(move || {
        for (seq, image) in receiver {
            let (width, height) = (image.width(), image.height());
            let rgb = image::DynamicImage::ImageRgba8(image).to_rgb8();
            let mut jpeg = Vec::with_capacity(64 * 1024);
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality);
            if rgb.write_with_encoder(encoder).is_ok() {
                out::frame(seq, width, height, &jpeg);
            }
        }
    });
    sender
}

fn read_commands(sender: mpsc::Sender<Message>) {
    use std::io::BufRead;
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Ok(value) = serde_json::from_str::<Value>(&line)
                && sender.send(Message::Command(value)).is_err()
            {
                return;
            }
        }
        let _ = sender.send(Message::Eof);
    });
}

fn main() {
    out::init();
    let args = parse_args();
    let (sender, receiver) = mpsc::channel::<Message>();

    let context =
        match SoftwareRenderingContext::new(dpi::PhysicalSize::new(args.width, args.height)) {
            Ok(context) => Rc::new(context),
            Err(error) => {
                eprintln!("orochi-servo: cannot create the rendering context: {error:?}");
                std::process::exit(1);
            }
        };
    if let Err(error) = context.make_current() {
        eprintln!("orochi-servo: cannot create the rendering context: {error:?}");
        std::process::exit(1);
    }
    if let Some(profile) = &args.profile
        && let Err(error) = std::fs::create_dir_all(profile)
    {
        eprintln!("orochi-servo: cannot create the profile directory: {error}");
        std::process::exit(1);
    }
    let opts = Opts {
        config_dir: args.profile.clone(),
        temporary_storage: args.temporary,
        ..Opts::default()
    };
    let preferences = servo::Preferences {
        dom_exec_command_enabled: args.exec_command,
        ..servo::Preferences::default()
    };
    let servo = ServoBuilder::default()
        .opts(opts)
        .preferences(preferences)
        .event_loop_waker(Box::new(Waker(sender.clone())))
        .build();
    let delegate = Rc::new(Delegate::default());
    let webview = WebViewBuilder::new(&servo, context.clone())
        .url(url::Url::parse("about:blank").expect("static"))
        .delegate(delegate.clone())
        .build();
    read_commands(sender);

    let mut frames = Frames {
        sender: frame_worker(args.quality),
        seq: 0,
        last: Instant::now() - FRAME_INTERVAL,
    };
    let mut composition = input::Composition::default();
    let mut shown = true;
    let mut frames_on = args.frames;

    'main: loop {
        let busy = delegate.dirty.get() || delegate.animating.get();
        let wait = if busy {
            Duration::from_millis(4)
        } else {
            Duration::from_millis(40)
        };
        let mut pending = vec![];
        match receiver.recv_timeout(wait) {
            Ok(message) => pending.push(message),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        pending.extend(receiver.try_iter());
        for message in pending {
            let command = match message {
                Message::Wake => continue,
                Message::Eof => break 'main,
                Message::Command(command) => command,
            };
            match command["cmd"].as_str().unwrap_or("") {
                "open" | "navigate" => {
                    let address = command["url"].as_str().unwrap_or("");
                    match url::Url::parse(address) {
                        Ok(url) if allowed(address) => webview.load(url),
                        _ => out::event(json!({"event": "refused", "url": address})),
                    }
                }
                "back" => {
                    if webview.can_go_back() {
                        webview.go_back(1);
                    }
                }
                "forward" => {
                    if webview.can_go_forward() {
                        webview.go_forward(1);
                    }
                }
                "reload" => webview.reload(),
                "stop" => webview.evaluate_javascript("window.stop()", |_| {}),
                "show" => {
                    webview.show();
                    shown = true;
                    delegate.dirty.set(true);
                }
                "hide" => {
                    webview.hide();
                    shown = false;
                }
                "resize" => {
                    let number = |key: &str, default: f64| command[key].as_f64().unwrap_or(default);
                    let ratio = number("ratio", 1.0).clamp(1.0, 3.0) as f32;
                    let width = (number("width", 1024.0).clamp(100.0, 4000.0) * f64::from(ratio))
                        .round() as u32;
                    let height = (number("height", 768.0).clamp(100.0, 4000.0) * f64::from(ratio))
                        .round() as u32;
                    webview.set_hidpi_scale_factor(euclid::Scale::new(ratio));
                    webview.resize(dpi::PhysicalSize::new(width, height));
                    delegate.dirty.set(true);
                }
                "zoom" => webview.set_page_zoom(
                    command["factor"].as_f64().unwrap_or(1.0).clamp(0.25, 5.0) as f32,
                ),
                "input" => {
                    for event in input::events(&command, &mut composition) {
                        webview.notify_input_event(event);
                    }
                }
                "dialog" => {
                    let dialog = delegate.dialog.borrow_mut().take();
                    if let Some(dialog) = dialog {
                        if command["accept"].as_bool().unwrap_or(false) {
                            match dialog {
                                SimpleDialog::Prompt(mut prompt) => {
                                    if let Some(text) = command["text"].as_str() {
                                        prompt.set_current_value(text);
                                    }
                                    prompt.confirm();
                                }
                                other => other.confirm(),
                            }
                        } else {
                            dialog.dismiss();
                        }
                        out::event(json!({"event": "dialog_closed"}));
                    }
                }
                "eval" => {
                    let id = command["id"].as_u64().unwrap_or(0);
                    let script = command["script"].as_str().unwrap_or("").to_owned();
                    webview.evaluate_javascript(script, move |result| {
                        out::event(match result {
                            Ok(value) => json!({"event": "eval", "id": id, "ok": true,
                                                "value": js_json(value)}),
                            Err(error) => json!({"event": "eval", "id": id, "ok": false,
                                                 "error": format!("{error:?}")}),
                        });
                    });
                }
                "screenshot" => {
                    let id = command["id"].as_u64().unwrap_or(0);
                    let size = context.size();
                    let rect = servo::DeviceIntRect::from_size(servo::DeviceIntSize::new(
                        size.width as i32,
                        size.height as i32,
                    ));
                    match context.read_to_image(rect) {
                        Some(image) => {
                            // A clip is in device pixels, kept inside the picture.
                            let image = match (command["x"].as_f64(), command["y"].as_f64()) {
                                (Some(x), Some(y)) => {
                                    let x =
                                        (x.max(0.0) as u32).min(image.width().saturating_sub(1));
                                    let y =
                                        (y.max(0.0) as u32).min(image.height().saturating_sub(1));
                                    let width = (command["width"].as_f64().unwrap_or(1.0).max(1.0)
                                        as u32)
                                        .min(image.width() - x);
                                    let height =
                                        (command["height"].as_f64().unwrap_or(1.0).max(1.0) as u32)
                                            .min(image.height() - y);
                                    image::imageops::crop_imm(&image, x, y, width, height)
                                        .to_image()
                                }
                                _ => image,
                            };
                            let mut png = std::io::Cursor::new(Vec::new());
                            let ok = image.write_to(&mut png, image::ImageFormat::Png).is_ok();
                            if ok {
                                out::blob("screenshot", id, png.get_ref());
                            } else {
                                out::event(json!({"event": "screenshot", "id": id, "len": 0}));
                            }
                        }
                        None => out::event(json!({"event": "screenshot", "id": id, "len": 0})),
                    }
                }
                "frames" => {
                    frames_on = command["on"].as_bool().unwrap_or(false);
                    delegate.dirty.set(frames_on);
                }
                "shutdown" => break 'main,
                _ => {}
            }
        }
        servo.spin_event_loop();

        if shown && frames_on && delegate.dirty.get() && frames.last.elapsed() >= FRAME_INTERVAL {
            let size = context.size();
            let rect = servo::DeviceIntRect::from_size(servo::DeviceIntSize::new(
                size.width as i32,
                size.height as i32,
            ));
            if let Some(image) = context.read_to_image(rect) {
                frames.seq += 1;
                if frames.sender.try_send((frames.seq, image)).is_ok() {
                    delegate.dirty.set(false);
                    frames.last = Instant::now();
                } else {
                    frames.seq -= 1;
                }
            }
        }
    }
    // Dropping Servo asks its threads to exit and waits for them, which is also what writes
    // the cookie jar and storage under the profile.
    drop(webview);
    drop(servo);
    std::process::exit(0);
}
