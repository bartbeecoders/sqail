//! The SQL editor driven with real key events (egui_kittest): keys that the
//! egui text editor turns into deletions must not delete text (#141).

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use sqail_client::proto::Engine;
use sqail_ui::commands::{Command, Keymap, parse_shortcut};
use sqail_ui::editor::{EditorEnv, Tab, editor_ui};
use sqail_ui::sql::format::Style;

/// Select bytes `sel` of `text`, press `key`, return the text after.
fn press(text: &str, sel: (usize, usize), modifiers: Modifiers, key: Key) -> String {
    let style = Style {
        uppercase: true,
        indent: 4,
    };
    let mut tab = Tab::new(1, "q".into(), None);
    tab.text = text.into();
    tab.select_bytes(sel.0, sel.1);
    let mut h = Harness::new_ui_state(
        move |ui, tab: &mut Tab| {
            let env = EditorEnv {
                engine: Some(Engine::Postgres),
                dark: false,
                font_size: 14.0,
                catalog: None,
                autocomplete: true,
                format: &style,
            };
            editor_ui(ui, tab, &env);
        },
        tab,
    );
    h.run_steps(2);
    h.key_press_modifiers(modifiers, key);
    h.run_steps(2);
    h.state().text.clone()
}

#[test]
fn tab_and_shift_tab_indent_the_selected_lines() {
    let text = "SELECT a\nFROM t\nWHERE x = 1";
    assert_eq!(
        press(text, (0, text.len()), Modifiers::NONE, Key::Tab),
        "\tSELECT a\n\tFROM t\n\tWHERE x = 1"
    );
    let indented = "\tSELECT a\n    FROM t\n  WHERE x = 1";
    assert_eq!(
        press(indented, (0, indented.len()), Modifiers::SHIFT, Key::Tab),
        text
    );
    // Without a selection Tab still types a tab.
    assert_eq!(
        press("SELECT a", (6, 6), Modifiers::NONE, Key::Tab),
        "SELECT\t a"
    );
}

#[test]
fn ctrl_h_k_u_w_do_not_delete() {
    let text = "SELECT a FROM t WHERE x = 1";
    for key in [Key::H, Key::K, Key::U, Key::W] {
        assert_eq!(
            press(text, (9, 9), Modifiers::CTRL, key),
            text,
            "Ctrl+{key:?}"
        );
    }
}

#[test]
fn ctrl_slash_toggles_a_comment() {
    let text = "SELECT a\nFROM t";
    let commented = press(text, (0, text.len()), Modifiers::COMMAND, Key::Slash);
    assert_eq!(commented, "-- SELECT a\n-- FROM t");
    assert_eq!(
        press(
            &commented,
            (0, commented.len()),
            Modifiers::COMMAND,
            Key::Slash
        ),
        text
    );
}

#[test]
fn ctrl_h_opens_find_and_replace() {
    let ctrl_h = parse_shortcut("Ctrl+H").unwrap();
    assert!(
        Keymap::defaults()
            .bindings()
            .iter()
            .any(|&(sc, cmd)| sc == ctrl_h && cmd == Command::Find)
    );
}
