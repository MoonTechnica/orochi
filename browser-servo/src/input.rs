//! `view_input`'s CDP-shaped fields as Servo input events.
use serde_json::Value;
use servo::{
    Code, CompositionEvent, CompositionState, ImeEvent, InputEvent, Key, KeyState, KeyboardEvent,
    Modifiers, MouseButton, MouseButtonAction, MouseButtonEvent, MouseMoveEvent, WebViewPoint,
    WheelDelta, WheelEvent, WheelMode,
};
use std::str::FromStr;

pub fn modifiers(bits: i64) -> Modifiers {
    let mut mods = Modifiers::empty();
    for (bit, flag) in [
        (1, Modifiers::ALT),
        (2, Modifiers::CONTROL),
        (4, Modifiers::META),
        (8, Modifiers::SHIFT),
    ] {
        if bits & bit != 0 {
            mods |= flag;
        }
    }
    mods
}

fn point(args: &Value) -> WebViewPoint {
    WebViewPoint::Page(euclid::Point2D::new(
        args["x"].as_f64().unwrap_or(0.0) as f32,
        args["y"].as_f64().unwrap_or(0.0) as f32,
    ))
}

fn button(name: &str) -> MouseButton {
    match name {
        "middle" => MouseButton::Auxiliary,
        "right" => MouseButton::Secondary,
        "back" => MouseButton::Back,
        "forward" => MouseButton::Forward,
        _ => MouseButton::Primary,
    }
}

fn key_event(state: KeyState, key: Key, code: Code, modifiers: Modifiers) -> InputEvent {
    InputEvent::Keyboard(KeyboardEvent::new(keyboard_types::KeyboardEvent {
        state,
        key,
        code,
        modifiers,
        ..Default::default()
    }))
}

#[derive(Default)]
pub struct Composition {
    active: bool,
}

pub fn events(args: &Value, composition: &mut Composition) -> Vec<InputEvent> {
    let string = |key: &str| args[key].as_str().unwrap_or("");
    let mods = modifiers(args["modifiers"].as_i64().unwrap_or(0));
    match string("type") {
        "mouse" => match string("kind") {
            "mouseMoved" => vec![InputEvent::MouseMove(MouseMoveEvent::new(point(args)))],
            "mousePressed" => vec![InputEvent::MouseButton(MouseButtonEvent::new(
                MouseButtonAction::Down,
                button(string("button")),
                point(args),
            ))],
            "mouseReleased" => vec![InputEvent::MouseButton(MouseButtonEvent::new(
                MouseButtonAction::Up,
                button(string("button")),
                point(args),
            ))],
            "mouseWheel" => vec![InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x: -args["deltaX"].as_f64().unwrap_or(0.0),
                    y: -args["deltaY"].as_f64().unwrap_or(0.0),
                    z: 0.0,
                    mode: WheelMode::DeltaPixel,
                },
                point(args),
            ))],
            _ => vec![],
        },
        "key" => {
            let text = string("text");
            let key = if text.is_empty() {
                Key::from_str(string("key")).unwrap_or(Key::Character(string("key").to_owned()))
            } else {
                Key::Character(text.to_owned())
            };
            let code = Code::from_str(string("code")).unwrap_or(Code::Unidentified);
            match string("kind") {
                "keyDown" | "rawKeyDown" => vec![key_event(KeyState::Down, key, code, mods)],
                "keyUp" => vec![key_event(KeyState::Up, key, code, mods)],
                "char" if !text.is_empty() => vec![
                    key_event(KeyState::Down, key.clone(), code, mods),
                    key_event(KeyState::Up, key, code, mods),
                ],
                _ => vec![],
            }
        }
        "text" => string("text")
            .chars()
            .flat_map(|c| {
                let key = if c == '\n' {
                    Key::Named(servo::NamedKey::Enter)
                } else {
                    Key::Character(c.to_string())
                };
                [
                    key_event(
                        KeyState::Down,
                        key.clone(),
                        Code::Unidentified,
                        Modifiers::empty(),
                    ),
                    key_event(KeyState::Up, key, Code::Unidentified, Modifiers::empty()),
                ]
            })
            .collect(),
        "ime" => {
            // A contenteditable inserts each Update instead of replacing the last, so only
            // the committed text is delivered, as one composition.
            if !args["commit"].as_bool().unwrap_or(false) {
                return vec![];
            }
            let text = string("text").to_owned();
            let composition_event = |state, data: String| {
                InputEvent::Ime(ImeEvent::Composition(CompositionEvent { state, data }))
            };
            composition.active = false;
            vec![
                composition_event(CompositionState::Start, String::new()),
                composition_event(CompositionState::Update, text.clone()),
                composition_event(CompositionState::End, text),
            ]
        }
        _ => vec![],
    }
}
