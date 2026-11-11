//! Undo in the SQL editor (#145): edits the app makes are one undo step each
//! and keep the typing before them; typing undoes word by word.

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use sqail_client::proto::Engine;
use sqail_ui::editor::{EditorEnv, Tab, editor_ui};
use sqail_ui::sql::format::Style;

fn editor(text: &str) -> Harness<'static, Tab> {
    let mut tab = Tab::new(1, "q".into(), None);
    tab.text = text.into();
    tab.select_bytes(text.len(), text.len());
    let mut h = Harness::new_ui_state(
        |ui, tab: &mut Tab| {
            let style = Style {
                uppercase: true,
                indent: 4,
            };
            let env = EditorEnv {
                engine: Some(Engine::Postgres),
                dark: false,
                font_size: 14.0,
                catalog: None,
                autocomplete: false,
                format: &style,
            };
            editor_ui(ui, tab, &env);
        },
        tab,
    );
    h.run_steps(2);
    h
}

/// Type `s` a character per frame, with no pause.
fn type_text(h: &mut Harness<'_, Tab>, s: &str) {
    for c in s.chars() {
        h.event(Event::Text(c.to_string()));
        h.step();
    }
}

fn press(h: &mut Harness<'_, Tab>, modifiers: Modifiers, key: Key) -> String {
    h.key_press_modifiers(modifiers, key);
    h.step();
    h.state().text.clone()
}

fn undo(h: &mut Harness<'_, Tab>) -> String {
    press(h, Modifiers::COMMAND, Key::Z)
}

#[test]
fn an_app_edit_is_one_step_and_keeps_the_typing_before_it() {
    let mut h = editor("");
    type_text(&mut h, "select 1");
    assert_eq!(h.state().text, "select 1");
    // What Format does, right after the typing (no pause in between).
    {
        let tab = h.state_mut();
        tab.undo_point();
        tab.text = "SELECT 1".into();
        tab.select_bytes(0, 8);
        tab.undo_point();
    }
    h.step();
    assert_eq!(undo(&mut h), "select 1", "the format alone is undone");
    assert_eq!(undo(&mut h), "select", "then the typing, a word at a time");
    assert_eq!(undo(&mut h), "");
    assert_eq!(undo(&mut h), "", "nothing left to undo");
    assert_eq!(press(&mut h, Modifiers::COMMAND, Key::Y), "select");
    assert_eq!(
        press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
        "select 1"
    );
    assert_eq!(press(&mut h, Modifiers::COMMAND, Key::Y), "SELECT 1");
}

#[test]
fn an_insert_is_one_step() {
    let mut h = editor("SELECT ");
    type_text(&mut h, "a, ");
    h.state_mut().insert_at_cursor("count(*)");
    h.step();
    type_text(&mut h, " AS n");
    assert_eq!(h.state().text, "SELECT a, count(*) AS n");
    assert_eq!(undo(&mut h), "SELECT a, count(*) AS");
    assert_eq!(undo(&mut h), "SELECT a, count(*)");
    assert_eq!(undo(&mut h), "SELECT a, ", "the insert alone");
    assert_eq!(undo(&mut h), "SELECT a,");
    assert_eq!(undo(&mut h), "SELECT ");
}

#[test]
fn indent_is_one_step() {
    let text = "SELECT a\nFROM t";
    let mut h = editor(text);
    type_text(&mut h, "x");
    h.state_mut().select_bytes(0, text.len() + 1);
    h.step();
    assert_eq!(
        press(&mut h, Modifiers::NONE, Key::Tab),
        "\tSELECT a\n\tFROM tx"
    );
    assert_eq!(undo(&mut h), "SELECT a\nFROM tx");
    assert_eq!(undo(&mut h), text);
}

#[test]
fn long_typing_undoes_word_by_word() {
    let mut h = editor("");
    let words: Vec<String> = (0..40).map(|i| format!("w{i}")).collect();
    // 40 words, about 30 s of frames, never a pause.
    type_text(&mut h, &words.join(" "));
    assert_eq!(undo(&mut h), words[..39].join(" "));
    assert_eq!(undo(&mut h), words[..38].join(" "));
}

#[test]
fn a_pause_and_a_cursor_move_start_new_steps() {
    let mut h = editor("");
    type_text(&mut h, "abc");
    h.run_steps(6); // 1.5 s
    type_text(&mut h, "def");
    assert_eq!(undo(&mut h), "abc");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    h.step();
    h.key_press(Key::Home);
    h.step();
    type_text(&mut h, "x");
    assert_eq!(h.state().text, "xabcdef");
    assert_eq!(undo(&mut h), "abcdef");
}
