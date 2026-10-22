export function transcriptChunks() {
  let modules;
  return (id, graph) => {
    if (!modules) {
      const ids = [...graph.getModuleIds()];
      const include = (root, seen) => {
        if (seen.has(root)) return;
        seen.add(root);
        for (const dependency of graph.getModuleInfo(root)?.importedIds ?? []) include(dependency, seen);
      };
      const shell = new Set(), eager = new Set();
      for (const root of ids.filter(key => graph.getModuleInfo(key)?.isEntry)) include(root, shell);
      const pane = ids.find(key => key.endsWith('/src/messages/MessagePane.tsx'));
      if (pane) include(pane, eager);
      modules = new Set([...eager].filter(key => !shell.has(key) && !/\/src\/messages\/(MessagePane\.tsx|compose\/(Composer|UploadDock)\.tsx)$/u.test(key)));
    }
    return modules.has(id) ? 'message-transcript' : undefined;
  };
}
