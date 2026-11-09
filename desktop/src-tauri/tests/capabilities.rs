//! The window may use the system's folder picker. Without a capability naming it Tauri refuses
//! the plugin's command, `Choose a folder…` does nothing, and a project cannot be added from
//! the window.
#[test]
fn the_window_is_allowed_the_folder_picker_a_project_is_chosen_with() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/capabilities/default.json"
    ))
    .expect("capabilities/default.json");
    let capability: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        capability["windows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w == "main"),
        "it names the window"
    );
    assert!(
        capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "dialog:allow-open"),
        "and lets it open the picker"
    );
}
