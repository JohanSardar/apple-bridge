import Foundation
import SourceSupport

extension NotesSource {
    static let extrasOperations = ["notes.move", "notes.rename_folder"]
    func extrasCapabilities() -> [String] {
        let url = deletionProofURL.deletingLastPathComponent().appendingPathComponent("notes-extras-proofs.json")
        guard let bytes = try? Data(contentsOf: url), let ledger = try? JSONDecoder().decode(JSON.self, from: bytes),
              ledger["version"].number == 1,
              ledger["os_build"].string == ProcessInfo.processInfo.operatingSystemVersionString,
              let generation = try? store.read(keys: []).generation, ledger["generation"].string == generation else { return [] }
        return Self.extrasOperations.filter { operation in
            let evidence = ledger["operations"][operation]
            return evidence["before_active"].bool && evidence["command_completed"].bool && evidence["notestore_verified"].bool && evidence["after_matches"].bool && evidence["cleanup_complete"].bool
        }
    }
    func requireExtrasProof(_ operation: String) throws {
        if Self.extrasOperations.contains(operation), !extrasCapabilities().contains(operation) { throw SourceFailure.capabilityUnproven }
    }
}
