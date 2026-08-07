import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import CrunchCatPng from "./assets/CrunchCat.png";

type Phase = "idle" | "drag-over" | "processing" | "done";

function App() {
  const [phase, setPhase] = useState<Phase>("idle");
  const [showShortcutBanner, setShowShortcutBanner] = useState(false);

  // Ask the backend whether the first-run question was already answered.
  useEffect(() => {
    invoke<boolean>("shortcut_state")
      .then((answered) => setShowShortcutBanner(!answered))
      .catch(() => setShowShortcutBanner(true));
  }, []);

  // Window-level drag & drop events (dropping onto the widget itself).
  useEffect(() => {
    const unlisten = getCurrentWindow().onDragDropEvent((event) => {
      const drag = event.payload;
      if (drag.type === "enter") {
        setPhase("drag-over");
      } else if (drag.type === "leave") {
        setPhase((p) => (p === "drag-over" ? "idle" : p));
      } else if (drag.type === "drop") {
        setPhase("processing");
        const path = drag.paths[0];
        invoke<string>("process_dropped_path", { path })
          .then(() => {
            setPhase("done");
            setTimeout(() => setPhase("idle"), 2000);
          })
          .catch(() => setPhase("idle"));
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // 'Yes': create the Desktop shortcut (backend spawns the slow copy on a
// background thread), then close the window immediately in `finally` —
// the window must vanish the instant the button is clicked.
const onYes = async () => {
  try {
    await invoke("create_desktop_shortcut");
  } catch (e) {
    console.error(e);
  } finally {
    await getCurrentWindow().close();
  }
};

// 'No': dismiss the shortcut question, then close the window immediately.
const onNo = async () => {
  try {
    await invoke("dismiss_shortcut");
  } catch (e) {
    console.error(e);
  } finally {
    await getCurrentWindow().close();
  }
};

const statusText =
    phase === "drag-over"
      ? "Release to extract"
      : phase === "processing"
        ? "Extracting…"
        : phase === "done"
          ? "Done ✓"
          : "Ready";

  return (
    <div className="app" data-tauri-drag-region>
      <div className={`card phase-${phase}`} data-tauri-drag-region>
        <div className="content" data-tauri-drag-region>
          <div className="cat-wrap" data-tauri-drag-region>
            <img
              src={CrunchCatPng}
              alt="CrunchCat"
              className="cat"
              draggable={false}
            />
          </div>
          <p className="hint" data-tauri-drag-region>
            Drop files onto the Desktop icon to extract or compress (zip)
          </p>
        </div>

        <p className={`status phase-${phase}`} data-tauri-drag-region>
          {statusText}
        </p>

        {showShortcutBanner && (
          <div className="banner">
            <span>Create Desktop shortcut?</span>
            <div className="actions">
              <button className="btn yes" onClick={onYes}>
                Yes
              </button>
              <button className="btn no" onClick={onNo}>
                No
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

export default App;