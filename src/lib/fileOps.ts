import { save, open } from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { useEditorStore } from "../stores/editorStore";

const SQL_FILTER = {
  name: "SQL Files",
  extensions: ["sql"],
};

/** Save the active tab's content. If it already has a filePath, overwrite; otherwise show Save As dialog. */
export async function saveQuery(): Promise<void> {
  const store = useEditorStore.getState();
  const tab = store.getActiveTab();
  if (!tab || !tab.content) return;

  if (tab.filePath) {
    await writeTextFile(tab.filePath, tab.content);
  } else {
    await saveQueryAs();
  }
}

/** Always show the Save As dialog, then write. */
export async function saveQueryAs(): Promise<void> {
  const store = useEditorStore.getState();
  const tab = store.getActiveTab();
  if (!tab || !tab.content) return;

  const filePath = await save({
    filters: [SQL_FILTER],
    defaultPath: `${tab.title}.sql`,
  });

  if (!filePath) return; // user cancelled

  await writeTextFile(filePath, tab.content);
  store.setFilePath(tab.id, filePath);

  // Update tab title to the filename
  const fileName = filePath.split(/[/\\]/).pop() ?? filePath;
  store.renameTab(tab.id, fileName);
}

/** Show Open dialog, read the file, load into the current tab (if empty) or a new tab. */
export async function openQuery(): Promise<void> {
  const filePath = await open({
    filters: [SQL_FILTER],
    multiple: false,
  });

  if (!filePath) return; // user cancelled

  const content = await readTextFile(filePath as string);
  const store = useEditorStore.getState();
  const tab = store.getActiveTab();

  const fileName = (filePath as string).split(/[/\\]/).pop() ?? (filePath as string);

  // If current tab is empty, load into it; otherwise create a new tab
  if (tab && !tab.content.trim()) {
    store.setContent(tab.id, content);
    store.setFilePath(tab.id, filePath as string);
    store.renameTab(tab.id, fileName);
  } else {
    store.addTab();
    const newTab = store.getActiveTab();
    if (newTab) {
      store.setContent(newTab.id, content);
      store.setFilePath(newTab.id, filePath as string);
      store.renameTab(newTab.id, fileName);
    }
  }
}
