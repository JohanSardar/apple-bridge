import Foundation
import Darwin
import OSAKit

extension AppleTruth {
    @MainActor
    static func notes(_ verb: String, _ request: [String: Any]) throws -> [String: Any] {
        guard verb == "fixture" || verb == "observe", let language = OSALanguage(forName: "JavaScript") else { throw NotesTruthFailure.invalid }
        let tag = request["tag"] as? String ?? ""
        if verb == "fixture" {
            guard tag.hasPrefix("[ab-test "), tag.hasSuffix("]"), !tag.contains("\n"), tag.count > 11 else { throw NotesTruthFailure.invalid }
        }
        var input = request
        input["verb"] = verb
        let encoded = try JSONSerialization.data(withJSONObject: input, options: [.sortedKeys])
        guard let argument = String(data: encoded, encoding: .utf8) else { throw NotesTruthFailure.invalid }
        let source = "JSON.stringify((function(request) {\n" + #"""
        const app = Application('com.apple.Notes');
        const tag = request.tag || '';
        const action = request.action || '';
        function require(value) { if (!value) throw new Error('invalid_fixture'); }
        function fixtureFolder(folder) {
            const name = folder.name();
            return name === 'Bridge Test' || name === 'Bridge Test ' + tag || name.startsWith('Bridge Test ' + tag + ' ');
        }
        function taggedNotes() {
            return app.folders().filter(fixtureFolder).reduce((notes, folder) => notes.concat(folder.notes().filter(note => note.name().startsWith(tag))), []);
        }
        function html(text) {
            return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').split('\n').map(line => '<div>' + line + '</div>').join('');
        }
        if (request.verb === 'observe') {
            const selected = tag ? taggedNotes() : app.notes();
            return {status:'verified', records:selected.map(n => {
                const protectedNote = n.passwordProtected();
                return {id:n.id(), title:n.name(), plain_text:protectedNote ? '' : n.plaintext(), locked:protectedNote};
            })};
        }
        if (action === 'cleanup_created_folder') {
            require(request.created_by_run === true && request.native_key && request.title.startsWith('Bridge Test ' + tag + ' '));
            const folders = app.folders.whose({name:request.title})();
            require(folders.length === 1);
            const folder = folders[0];
            require(folder.folders().length === 0 && folder.notes().every(n => n.name().startsWith(tag)));
            const parent = folder.container();
            const folderID = folder.id();
            app.delete(parent.folders.byId(folderID));
            require(parent.folders.whose({id:folderID})().length === 0);
            return {status:'verified'};
        }
        if (action === 'cleanup' || action === 'delete') {
            taggedNotes().forEach(note => {
                const id = note.id();
                require(note.name().startsWith(tag));
                const folder = note.container();
                require(fixtureFolder(folder));
                app.delete(folder.notes.byId(id));
            });
            require(taggedNotes().length === 0);
            return {status:'verified'};
        }
        if (action === 'modify') {
            require(request.id && request.plain_text.startsWith(tag));
            const note = app.notes.byId(request.id);
            require(note.name().startsWith(tag));
            note.body = html(request.plain_text);
            return {status:'verified'};
        }
        require(action === 'create' && request.title.startsWith(tag));
        let folders = app.folders.whose({name:'Bridge Test'})();
        let created = false;
        if (folders.length === 0) {
            const folder = app.Folder({name:'Bridge Test'});
            app.folders.push(folder);
            folders = [folder];
            created = true;
        }
        const folder = folders[0];
        const body = request.html || '<div>' + request.title + '</div><div>synthetic body</div>';
        require(body.includes(tag));
        const note = app.Note({body:body});
        folder.notes.push(note);
        return {status:'verified', id:note.id(), folder_id:folder.id(), folder_created:created};
        """# + "\n})(" + argument + "))"
        var error: NSDictionary?
        let result = OSAScript(source: source, language: language).executeAndReturnError(&error)
        if let error { throw NSError(domain: "NotesJavaScript", code: error[OSAScriptErrorNumber] as? Int ?? 0, userInfo: [NSLocalizedDescriptionKey: error[OSAScriptErrorMessage] as? String ?? "automation_failed"]) }
        guard let text = result?.stringValue, let bytes = text.data(using: .utf8), var response = try JSONSerialization.jsonObject(with: bytes) as? [String: Any] else { throw NotesTruthFailure.invalid }
        if verb == "fixture" { response["effect_uptime_ns"] = clock_gettime_nsec_np(CLOCK_UPTIME_RAW) }
        return response
    }
}

private enum NotesTruthFailure: Error { case invalid }
