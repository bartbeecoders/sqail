//! Organising the connection tree: folders, in-place renames and moving
//! connections between folders.
//!
//! A folder is the `folder` field of the connections in it, stored on the
//! service. A folder with no connections yet (just created, or emptied) only
//! exists locally, in `settings.toml`, per service, until a connection is
//! moved into it.

use sqail_client::proto::{Connection, ConnectionInput};
use uuid::Uuid;

use crate::app::{Msg, SqailApp};

/// What is being renamed in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameTarget {
    Connection(Uuid),
    Folder(String),
}

#[derive(Debug, Clone)]
pub struct Rename {
    pub target: RenameTarget,
    pub text: String,
    /// Focus the field and select its text on the next frame.
    pub focus: bool,
}

impl Rename {
    pub fn new(target: RenameTarget, current: &str) -> Self {
        Self {
            target,
            text: current.to_string(),
            focus: true,
        }
    }
}

/// Drag payload of a connection row.
#[derive(Debug, Clone, Copy)]
pub struct ConnDrag(pub Uuid);

/// The profile of `c` as an update body that keeps its stored secrets.
pub fn input_of(c: &Connection) -> ConnectionInput {
    ConnectionInput {
        name: c.name.clone(),
        params: c.params.clone(),
        password: None,
        ssl_client_key: None,
        read_only: c.read_only,
        color: c.color.clone(),
        environment: c.environment.clone(),
        folder: c.folder.clone(),
    }
}

fn service_key(app: &SqailApp) -> Option<String> {
    app.service.profile.as_ref().map(|p| p.url.clone())
}

/// Folders of this service that have no connections (kept locally).
pub fn local_folders(app: &SqailApp) -> Vec<String> {
    service_key(app)
        .and_then(|k| app.settings.folders.get(&k).cloned())
        .unwrap_or_default()
}

fn set_local_folders(app: &mut SqailApp, mut folders: Vec<String>) {
    let Some(key) = service_key(app) else {
        return;
    };
    folders.sort();
    folders.dedup();
    if folders.is_empty() {
        app.settings.folders.remove(&key);
    } else {
        app.settings.folders.insert(key, folders);
    }
    app.settings.save();
}

/// Every folder: those of the connections and the local empty ones, sorted.
pub fn all_folders(app: &SqailApp) -> Vec<String> {
    let mut all: Vec<String> = app
        .service
        .connections
        .iter()
        .filter_map(|c| c.folder.clone())
        .chain(local_folders(app))
        .collect();
    all.sort_by_key(|f| f.to_lowercase());
    all.dedup();
    all
}

/// `New folder`, or `New folder 2`, … when taken.
fn new_folder_name(taken: &[String]) -> String {
    (1..)
        .map(|n| match n {
            1 => "New folder".to_string(),
            n => format!("New folder {n}"),
        })
        .find(|c| !taken.iter().any(|t| t.eq_ignore_ascii_case(c)))
        .unwrap_or_default()
}

/// Add an empty folder and start renaming it.
pub fn new_folder(app: &mut SqailApp) {
    let name = new_folder_name(&all_folders(app));
    let mut local = local_folders(app);
    local.push(name.clone());
    set_local_folders(app, local);
    app.schema.rename = Some(Rename::new(RenameTarget::Folder(name.clone()), &name));
}

/// Remove an empty local folder. Folders with connections disappear when
/// their last connection moves out.
pub fn delete_folder(app: &mut SqailApp, name: &str) {
    let local = local_folders(app)
        .into_iter()
        .filter(|f| f != name)
        .collect();
    set_local_folders(app, local);
}

/// Apply the rename being edited. An empty or unchanged name cancels it.
pub fn commit_rename(app: &mut SqailApp) {
    let Some(r) = app.schema.rename.take() else {
        return;
    };
    let text = r.text.trim().to_string();
    if text.is_empty() {
        return;
    }
    match r.target {
        RenameTarget::Connection(id) => {
            let Some(c) = app.service.connection(id) else {
                return;
            };
            if c.name == text {
                return;
            }
            let mut input = input_of(c);
            input.name = text;
            save(app, vec![(id, input)]);
        }
        RenameTarget::Folder(old) => {
            if old == text {
                return;
            }
            // Renaming onto an existing folder merges the two.
            let local = local_folders(app)
                .into_iter()
                .map(|f| if f == old { text.clone() } else { f })
                .collect();
            set_local_folders(app, local);
            let edits = app
                .service
                .connections
                .iter()
                .filter(|c| c.folder.as_deref() == Some(old.as_str()))
                .map(|c| {
                    let mut input = input_of(c);
                    input.folder = Some(text.clone());
                    (c.id, input)
                })
                .collect();
            save(app, edits);
        }
    }
}

/// Move a connection into `folder` (`None`: out of every folder).
pub fn move_connection(app: &mut SqailApp, id: Uuid, folder: Option<String>) {
    let Some(c) = app.service.connection(id).cloned() else {
        return;
    };
    if c.folder == folder {
        return;
    }
    // The folder it leaves stays, empty, until the user deletes it.
    if let Some(old) = c.folder.clone()
        && !app
            .service
            .connections
            .iter()
            .any(|o| o.id != id && o.folder.as_deref() == Some(old.as_str()))
    {
        let mut local = local_folders(app);
        local.push(old);
        set_local_folders(app, local);
    }
    let mut input = input_of(&c);
    input.folder = folder;
    save(app, vec![(id, input)]);
}

/// Update profiles on the service. The tree shows the change at once; the
/// list is reloaded when the service has answered.
fn save(app: &mut SqailApp, edits: Vec<(Uuid, ConnectionInput)>) {
    if edits.is_empty() {
        return;
    }
    for (id, input) in &edits {
        if let Some(c) = app.service.connections.iter_mut().find(|c| c.id == *id) {
            c.name = input.name.clone();
            c.folder = input.folder.clone();
        }
    }
    // A folder that now has connections no longer needs a local entry.
    let used: Vec<String> = app
        .service
        .connections
        .iter()
        .filter_map(|c| c.folder.clone())
        .collect();
    let local = local_folders(app)
        .into_iter()
        .filter(|f| !used.contains(f))
        .collect();
    set_local_folders(app, local);
    let Some(client) = app.service.client.clone() else {
        return;
    };
    app.worker.run(async move {
        let mut errors = Vec::new();
        for (id, input) in edits {
            if let Err(e) = client.update_connection(id, &input).await {
                errors.push(format!("{}: {e}", input.name));
            }
        }
        Msg::TreeSaved(errors)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_folder_names_count_up() {
        assert_eq!(new_folder_name(&[]), "New folder");
        assert_eq!(
            new_folder_name(&["new folder".into(), "New folder 2".into()]),
            "New folder 3"
        );
    }
}
