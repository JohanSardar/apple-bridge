import Foundation
import CoreGraphics
import Darwin
import SourceSupport
import MessagesSource

@MainActor
public enum MessagesSendVariantProof {
    public static func run() async throws -> JSON {
        let lockHome=PrivateReport.directory.appendingPathComponent("messages-selftest",isDirectory:true)
        try FileManager.default.createDirectory(at:lockHome,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
        let descriptor=Darwin.open(lockHome.appendingPathComponent("selftest.lock").path,O_CREAT|O_RDWR|O_NOFOLLOW,0o600)
        guard descriptor>=0 else { throw SourceFailure.unavailable }
        defer { Darwin.close(descriptor) }
        var info=stat()
        guard fstat(descriptor,&info)==0,info.st_uid==getuid(),info.st_mode & 0o777==0o600,flock(descriptor,LOCK_EX|LOCK_NB)==0 else { throw SourceFailure.conflict }
        let build=ProcessInfo.processInfo.operatingSystemVersionString,run="variants_"+UUID().uuidString.replacingOccurrences(of:"-",with:"")
        let tag="[ab-test "+run+"]",directory=PrivateReport.directory.appendingPathComponent("messages-command-cleanup",isDirectory:true)
        let own=Set(try MessagesProbes.discoverSelfHandles().handles)
        let database=try MessagesDatabase()
        let existing=try database.rows("SELECT c.guid,h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.service_name='iMessage' AND (SELECT COUNT(*) FROM chat_handle_join x WHERE x.chat_id=c.ROWID)=1 ORDER BY c.ROWID DESC")
        let proofBytes=try Data(contentsOf:PrivateReport.directory.appendingPathComponent("messages-command-proofs.json")),proof=try JSONDecoder().decode(JSON.self,from:proofBytes)
        guard proof["os_build"].string==build,let selfChat=existing.first(where:{ row in own.contains(MessagesProbes.normalize(row["id"] ?? "")) && proof["routes"].array.map(\.string).contains(MessagesSendVariants.routeKey(.object(["kind":.string("chat_id_raw"),"chat":.string(row["guid"] ?? "")]))+":text") }),let handle=selfChat["id"],let chat=selfChat["guid"] else { throw SourceFailure.capabilityUnproven }
        let baseline=Int64(try database.rows("SELECT COALESCE(MAX(ROWID),0) AS maximum FROM message").first?["maximum"] ?? "0") ?? 0
        try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
        let ledger=directory.appendingPathComponent(run+".json")
        var registration:[String:Any]=["scenario":"MSG-SEND-VARIANTS","run_id":run,"baseline_max_row_id":baseline,"media_attachment_names":[],"cleanup_state":"pending_native_cleanup"]
        try PrivateReport.write(registration,to:ledger)
        let guardHome=PrivateReport.directory.appendingPathComponent("messages-variant-selftest",isDirectory:true),commands=MessagesCommands(path:nil,labCommands:true),guardLog=try DispatchGuard(home:guardHome)
        var steps:[JSON]=[],lateEffectPossible=false
        let locked=(CGSessionCopyCurrentDictionary() as? [String:Any])?["CGSSessionScreenIsLocked"] as? Bool == true
        func save(complete: Bool = false) throws -> JSON {
            let result=JSON.object(["version":.number(1),"complete":.bool(complete),"os_build":.string(build),"helper_digest":.string(MessagesParity.buildDigest),"run_id":.string(run),"steps":.array(steps),"state":.string(steps.allSatisfy { $0["proven"].bool } ? "proven":"limited")])
            try JSONEncoder().encode(result).write(to:MessagesSendVariants.proofURL,options:.atomic)
            try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:MessagesSendVariants.proofURL.path)
            return result
        }
        if locked {
            steps=MessagesSendVariants.operations.map { .object(["operation":.string($0),"proven":.bool(false),"reason":.string("ui_deferred_until_unlock")]) }
            return try save(complete:true)
        }
        @MainActor func execute(_ text: String, variant: JSON? = nil) async throws -> (JSON,JSON) {
            let id=UUID().uuidString,expiry=Date().addingTimeInterval(45)
            var args:[String:JSON]=["text":.string(text)]
            if let variant { args["variant"]=variant }
            let frame=JSON.object(["command_id":.string(id),"principal":.string("selftest"),"request_hash":.string(UUID().uuidString),"expires_at":.string(timestamp(expiry)),"action":.object(["op":.string("messages.send"),"list_key":.string(chat),"messages":.object(args)])])
            guard !(await guardLog.contains(id)) else { throw SourceFailure.conflict }
            let prepared=try await commands.prepare(frame)
            try await commands.validate(prepared)
            guard Date()<expiry else { throw SourceFailure.expired }
            try await guardLog.append(.object(["t":.string("Started"),"command_id":.string(id),"prepared":.object(["action":prepared.action,"native_state":prepared.nativeState])]))
            let result:JSON
            do { result=try await commands.dispatch(prepared) }
            catch { if (error as? SendVariantFailure)==nil || (error as? SendVariantFailure) == .sendReceiptUnknown { lateEffectPossible=true };throw error }
            if result["disposition"].string=="indeterminate" { lateEffectPossible=true }
            try await guardLog.append(.object(["t":.string("Outcome"),"command_id":.string(id),"evidence":result,"disposition":result["disposition"]]))
            return (result,prepared.nativeState["destination"])
        }
        let seed:JSON
        do {
            let result=try await execute(tag+" send variant proof")
            guard result.0["observed"].bool,["locally_observed","delivered"].contains(result.0["disposition"].string) else { throw SourceFailure.unavailable }
            seed=result.0
        } catch {
            steps=MessagesSendVariants.operations.map { .object(["operation":.string($0),"proven":.bool(false),"reason":.string("tagged_self_seed_unproven")]) }
            _=try? await MessagesCleanup.run(["--tag",tag])
            return try save(complete:true)
        }
        var content=tag+" ",ranges:[JSON]=[]
        for name in MessagesSendVariants.styles+MessagesSendVariants.textEffects {
            let location=content.utf16.count;content+=name+" "
            ranges.append(.object(["utf16_location":.number(Double(location)),"utf16_length":.number(Double(name.utf16.count)),"styles":.array(MessagesSendVariants.styles.contains(name) ? [.string(name)]:[]),"effect":.string(MessagesSendVariants.textEffects.contains(name) ? name:"")]))
        }
        let variants:[JSON]=[
            .object(["kind":.string("schedule"),"scheduled_for":.string(timestamp(Date().addingTimeInterval(3600)))]),
            .object(["kind":.string("effect"),"effect":.string("loud")]),
            .object(["kind":.string("formatting"),"formatting":.array(ranges)]),
            .object(["kind":.string("mention"),"handle_id":.string("self"),"utf16_location":.number(Double(tag.utf16.count+1)),"utf16_length":.number(Double(handle.utf16.count+1))]),
            .object(["kind":.string("audio")])
        ]
        for (index,variant) in variants.enumerated() {
            let operation=MessagesSendVariants.operation(.object(["variant":variant]))!,text=variant["kind"].string=="formatting" ? content:variant["kind"].string=="mention" ? tag+" @"+handle:tag+" "+variant["kind"].string
            var reason="",proven=false,route:JSON = .null
            do {
                if variant["kind"].string=="audio" {
                    try await MessagesSendVariants.probeAudio(target:seed["native_key"].string)
                } else {
                    let result=try await execute(text,variant:variant);proven=result.0["observed"].bool && ["locally_observed","delivered"].contains(result.0["disposition"].string);route=result.1;reason=proven ? "proven_tagged_self_capture":result.0["reason"].string
                }
            } catch {
                reason=(error as? SendVariantFailure)?.rawValue ?? (error as? MessagesUIFailure)?.rawValue ?? String(describing:error)
            }
            if !proven,reason.isEmpty { reason="native_transition_unverified" }
            steps.append(.object(["operation":.string(operation),"proven":.bool(proven),"reason":.string(reason),"route":.string(route == .null ? "":MessagesSendVariants.routeKey(route)),"variant":variant,"attempt":.number(Double(index+1))]))
            _=try save()
            _=try? await MessagesSendVariants.clearProofDraft(tag)
        }
        steps.append(.object(["operation":.string("messages.keep_audio"),"proven":.bool(false),"reason":.string("keptReadbackUnavailable"),"attempt":.number(6)]))
        do {
            let result=try await MessagesCleanup.run(["--tag",tag]),cleanup=try JSONDecoder().decode(JSON.self,from:result)
            registration["cleanup_state"]=cleanup["passed"].bool ? (lateEffectPossible ? "pending_reconciliation":"verified_clean"):"pending_native_cleanup"
            registration["late_effect_possible"]=lateEffectPossible
            try PrivateReport.write(registration,to:ledger)
            if !cleanup["passed"].bool { steps=steps.map { step in var value=step.object;value["proven"] = .bool(false);value["proof_reason"] = value["reason"];value["reason"] = .string("tagged_cleanup_unverified");return .object(value) } }
        } catch {
            steps=steps.map { step in var value=step.object;value["proven"] = .bool(false);value["proof_reason"] = value["reason"];value["reason"] = .string("tagged_cleanup_unverified");return .object(value) }
        }
        return try save(complete:true)
    }
}
