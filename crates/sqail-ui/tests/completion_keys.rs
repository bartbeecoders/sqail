//! The SQL editor driven with real key events (egui_kittest): typing a
//! finished word and pressing Enter must give a new line, not a completion
//! (#140).

use egui::Key;
use egui_kittest::Harness;
use sqail_client::proto::{ColumnInfo, Engine, ForeignKeyInfo, TableInfo, TableKind};
use sqail_ui::editor::{EditorEnv, Tab, editor_ui};
use sqail_ui::sql::complete::Catalog;
use sqail_ui::sql::format::Style;

struct Fake;

impl Catalog for Fake {
    fn schemas(&self) -> Option<Vec<String>> {
        Some(vec!["public".into()])
    }
    fn tables(&self, _schema: &str) -> Option<Vec<TableInfo>> {
        let t = |n: &str| TableInfo {
            schema: Some("public".into()),
            name: n.into(),
            kind: TableKind::Table,
        };
        Some(vec![t("orders"), t("customers")])
    }
    fn columns(&self, _schema: &str, table: &str) -> Option<Vec<ColumnInfo>> {
        let c = |n: &str| ColumnInfo {
            name: n.into(),
            ordinal: 0,
            data_type: "int".into(),
            nullable: true,
            default: None,
            primary_key: false,
        };
        Some(match table {
            "orders" => vec![c("id"), c("customer_id"), c("status")],
            "customers" => vec![c("id"), c("name"), c("username")],
            _ => return None,
        })
    }
    fn foreign_keys(&self, _schema: &str, _table: &str) -> Option<Vec<ForeignKeyInfo>> {
        Some(vec![])
    }
}

/// Type `typed` after `start` key by key, then press Enter; the text after.
fn type_then_enter(start: &str, typed: &str) -> String {
    let style = Style {
        uppercase: true,
        indent: 4,
    };
    let mut tab = Tab::new(1, "q".into(), None);
    tab.text = start.into();
    tab.select_bytes(start.len(), start.len());
    let mut h = Harness::new_ui_state(
        move |ui, tab: &mut Tab| {
            let env = EditorEnv {
                engine: Some(Engine::Postgres),
                dark: false,
                font_size: 14.0,
                catalog: Some(&Fake),
                autocomplete: true,
                format: &style,
            };
            editor_ui(ui, tab, &env);
        },
        tab,
    );
    h.run_steps(2);
    for c in typed.chars() {
        h.event(egui::Event::Text(c.to_string()));
        h.run_steps(2);
    }
    h.key_press(Key::Enter);
    h.run_steps(2);
    h.state().text.clone()
}

#[test]
fn enter_after_a_whole_word_is_a_new_line() {
    for (start, typed) in [
        ("SELECT * FROM orders ORDER BY ", "id"),
        ("SELECT count(*) ", "AS"),
        ("SELECT * FROM customers WHERE ", "name"),
        ("SELECT * FROM orders -- get ", "ord"),
        ("SELECT * FROM orders WHERE status = '", "cust"),
        ("SELECT * FROM orders /* ", "cust"),
        // Only a match inside another name (`customer_id`).
        ("SELECT * FROM orders WHERE ", "omer"),
    ] {
        let want = format!("{start}{typed}\n");
        assert_eq!(type_then_enter(start, typed), want, "typing {typed:?}");
    }
}

#[test]
fn enter_still_completes_a_started_name() {
    assert_eq!(
        type_then_enter("SELECT * FROM orders WHERE ", "cust"),
        "SELECT * FROM orders WHERE customer_id"
    );
}
