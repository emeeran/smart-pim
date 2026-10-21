/**
 * Shared Tiptap editor with formatting toolbar (extracted from Notes so the
 * Diary module reuses it verbatim). Remounts on empty→content transitions;
 * callers key it per item for clean state.
 */
import { useEffect, useRef, useState } from "preact/hooks";
import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";

export function RichEditor({
  html,
  onChange,
}: {
  html: string;
  onChange: (html: string) => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const editor = useRef<Editor | null>(null);
  const [bold, setBold] = useState(false);
  const [italic, setItalic] = useState(false);

  useEffect(() => {
    if (!host.current) return;
    const ed = new Editor({
      element: host.current,
      extensions: [StarterKit],
      content: html,
      editorProps: {
        attributes: { class: "tiptap-content", spellcheck: "true" },
      },
    });
    const sync = () => {
      setBold(ed.isActive("bold"));
      setItalic(ed.isActive("italic"));
      onChange(ed.getHTML());
    };
    ed.on("update", sync);
    ed.on("selectionUpdate", sync);
    editor.current = ed;
    return () => {
      ed.destroy();
      editor.current = null;
    };
    // Editor is created once per item selection (html changes remount).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [html === "" ? -1 : 0]);

  const cmd = (fn: (ed: Editor) => void) => {
    const ed = editor.current;
    if (ed) {
      fn(ed);
      ed.commands.focus();
    }
  };

  return (
    <div class="rich-editor">
      <div class="editor-toolbar">
        <button
          data-active={bold}
          onMouseDown={(e) => {
            e.preventDefault();
            cmd((ed) => ed.chain().focus().toggleBold().run());
          }}
        >
          B
        </button>
        <button
          data-active={italic}
          onMouseDown={(e) => {
            e.preventDefault();
            cmd((ed) => ed.chain().focus().toggleItalic().run());
          }}
        >
          I
        </button>
        <button onMouseDown={(e) => { e.preventDefault(); cmd((ed) => ed.chain().focus().toggleHeading({ level: 2 }).run()); }}>H2</button>
        <button onMouseDown={(e) => { e.preventDefault(); cmd((ed) => ed.chain().focus().toggleBulletList().run()); }}>• List</button>
        <button onMouseDown={(e) => { e.preventDefault(); cmd((ed) => ed.chain().focus().toggleOrderedList().run()); }}>1. List</button>
        <button onMouseDown={(e) => { e.preventDefault(); cmd((ed) => ed.chain().focus().toggleBlockquote().run()); }}>❝</button>
        <button onMouseDown={(e) => { e.preventDefault(); cmd((ed) => ed.chain().focus().toggleCodeBlock().run()); }}>{"</>"}</button>
      </div>
      <div ref={host} />
    </div>
  );
}
