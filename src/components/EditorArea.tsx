import EditorTabs from "./EditorTabs";
import SqlEditor from "./SqlEditor";

interface EditorAreaProps {
  onExecute?: (sql: string) => void;
  onFormat?: () => void;
}

export default function EditorArea({ onExecute, onFormat }: EditorAreaProps) {
  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <EditorTabs />
      <SqlEditor onExecute={onExecute} onFormat={onFormat} />
    </div>
  );
}
