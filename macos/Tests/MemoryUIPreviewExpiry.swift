import Foundation

@main
struct MemoryUIPreviewExpiryTests {
    static func main() throws {
        let data = Data("""
        {"token":"fixture","expires_at":400.75,"action":"memory.pgvector.initialize.preview","runner_id":"runner","bot_id":"bot","profile_id":null,"profile_revision":null,"connection_revision":{"counter":1,"device_id":"fixture"},"details":{"target":{"database":"fixture","database_oid":42,"role":"fixture","server_address":null,"server_port":5432,"server_version":"17","session_pid":22},"schema":"fixture","sql":"SELECT 1;","readiness":{"ready":false,"extension_version":null,"extension_schema":null,"schema_exists":false,"can_create_schema":true,"can_create_tables":true,"can_install_extension":false}}}
        """.utf8)
        let preview = try JSONDecoder().decode(MemoryBotSetupPreview<MemoryPgvectorDetails>.self, from: data)
        let target = preview.target
        let approval = try MemoryBotSetupApproval(preview: preview, expected: .pgvector)
        _ = try approval.applyRequest(confirm: true, current: target, now: Date(timeIntervalSince1970: 400.5))
        let expired = try MemoryBotSetupApproval(preview: preview, expected: .pgvector)
        do {
            _ = try expired.applyRequest(confirm: true, current: target, now: Date(timeIntervalSince1970: 400.75))
            fatalError("expiry boundary accepted")
        } catch MemorySetupError.approvalExpired { }
        print("PASS: real core fractional expiry and exact expiry boundary")
    }
}
