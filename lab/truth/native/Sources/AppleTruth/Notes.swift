import Foundation
import Darwin
import OSAKit
import Carbon

extension AppleTruth {
    @MainActor
    static func notes(_ verb: String, _ request: [String: Any]) async throws -> [String: Any] {
        guard verb == "fixture" || verb == "observe", let language = OSALanguage(forName: "JavaScript") else { throw NotesTruthFailure.invalid }
        let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.Notes")
        guard AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false) == noErr else { throw TruthFailure.permissionMissing }
        let tag = request["tag"] as? String ?? ""
        if verb == "fixture" {
            guard tag.hasPrefix("[ab-test "), tag.hasSuffix("]"), !tag.contains("\n"), tag.count > 11 else { throw NotesTruthFailure.invalid }
        }
        let cleanup = verb == "fixture" && ["cleanup", "delete", "cleanup_owned", "cleanup_created_folder"].contains(request["action"] as? String ?? "")
        let storeTargets = cleanup && request["folder_id"] == nil ? try notesDeletionTargets(tag: tag, folderKey: request["action"] as? String == "cleanup_created_folder" ? request["native_key"] as? String : nil) : []
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
                return {id:n.id(), folder_id:n.container().id(), title:n.name(), plain_text:protectedNote ? '' : n.plaintext(), locked:protectedNote};
            })};
        }
        if (action === 'folder_identity') {
            require(request.title.startsWith('Bridge Test ' + tag));
            const folders = app.folders.whose({name:request.title})();
            require(folders.length === 1);
            return {status:'verified',id:folders[0].id()};
        }
        if (action === 'cleanup_owned') {
            require(request.id && request.folder_id);
            const note = app.notes.byId(request.id);
            require(note.name().startsWith(tag) && note.container().id() === request.folder_id && fixtureFolder(note.container()));
            return {status:'verified', deleted_ids:[request.id]};
        }
        if (action === 'cleanup_created_folder') {
            require(request.created_by_run === true && (request.native_key || request.folder_id) && (request.title === 'Bridge Test ' + tag || request.title.startsWith('Bridge Test ' + tag + ' ')));
            const folders = request.folder_id ? [app.folders.byId(request.folder_id)] : app.folders.whose({name:request.title})();
            require(folders.length === 1);
            const folder = folders[0];
            require(folder.name() === 'Bridge Test ' + tag || folder.name().startsWith('Bridge Test ' + tag + ' '));
            require((!request.folder_id || folder.id() === request.folder_id) && folder.folders().length === 0 && (request.folder_id ? folder.notes().length === 0 : folder.notes().every(n => n.name().startsWith(tag))));
            const folderID = folder.id();
            return {status:'verified', deleted_ids:[folderID]};
        }
        if (action === 'cleanup' || action === 'delete') {
            const selected = taggedNotes(), deletedIDs = selected.map(note => note.id());
            selected.forEach(note => {
                const id = note.id();
                require(note.name().startsWith(tag));
                const folder = note.container();
                require(fixtureFolder(folder));
            });
            return {status:'verified', deleted_ids:deletedIDs};
        }
        if (action === 'modify') {
            require(request.id && request.plain_text.startsWith(tag));
            const note = app.notes.byId(request.id);
            require(note.name().startsWith(tag));
            note.body = html(request.plain_text);
            return {status:'verified'};
        }
        require(action === 'create' && request.title.startsWith(tag));
        const folderName = request.folder_title || 'Bridge Test';
        require(folderName === 'Bridge Test' || folderName === 'Bridge Test ' + tag);
        let folders = app.folders.whose({name:folderName})();
        let created = false;
        if (folders.length === 0) {
            const folder = app.Folder({name:folderName});
            app.folders.push(folder);
            folders = app.folders.whose({name:folderName})();
            require(folders.length === 1);
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
        if verb == "observe", request["active_only"] as? Bool == true, let records = response["records"] as? [[String: Any]] {
            response["records"] = try records.filter { record in
                guard let id = record["id"] as? String else { throw NotesTruthFailure.invalid }
                return try !notesDeletionVerified([id])
            }
        }
        if cleanup {
            let deletedIDs = response.removeValue(forKey: "deleted_ids") as? [String] ?? []
            for id in deletedIDs {
                _ = try notesDeletionVerified([id])
                let kind = id.contains("/ICNote/") ? "note" : "folder"
                let quoted = "\"" + id.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
                guard let script = NSAppleScript(source: "with timeout of 5 seconds\ntell application id \"com.apple.Notes\" to delete " + kind + " id " + quoted + "\nend timeout") else { throw NotesTruthFailure.invalid }
                var failure: NSDictionary?
                script.executeAndReturnError(&failure)
                if let failure { throw NSError(domain: "NotesAppleScript", code: failure[OSAScriptErrorNumber] as? Int ?? 0, userInfo: [NSLocalizedDescriptionKey: "owned_cleanup_failed"]) }
            }
            let ids = Array(Set(storeTargets + deletedIDs))
            var verified = false
            for _ in 0..<600 {
                if try notesDeletionVerified(ids) { verified = true; break }
                try await Task.sleep(for: .milliseconds(50))
            }
            guard verified else { throw NSError(domain: "NotesCleanup", code: 1, userInfo: [NSLocalizedDescriptionKey: "notestore_deletion_unverified"]) }
        }
        if verb == "fixture" { response["effect_uptime_ns"] = clock_gettime_nsec_np(CLOCK_UPTIME_RAW) }
        return response
    }
}

private enum NotesTruthFailure: Error { case invalid }
