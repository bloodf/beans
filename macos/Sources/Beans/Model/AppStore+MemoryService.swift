import Foundation

extension AppStore {
    var memoryService: MemoryServiceAPI {
        MemoryServiceAPI { [weak self] method, parameters in
            guard let self else { throw MemoryUIError.unavailable }
            return try await self.memoryRequest(method, parameters: parameters)
        }
    }

    var memorySetup: MemorySetupAPI {
        MemorySetupAPI { [weak self] method, parameters in
            guard let self else { throw MemoryUIError.unavailable }
            return try await self.memoryRequest(method, parameters: parameters)
        }
    }

    private func memoryRequest(_ method: String, parameters: Data) async throws -> Data {
        guard MemoryUIRPC.allowed.contains(method) else { throw MemoryUIError.unsupported }
        guard !isMock, hasIdentity == true, isConnected else { throw MemoryUIError.unavailable }
        guard let object = try JSONSerialization.jsonObject(with: parameters) as? [String: Any] else {
            throw MemoryUIError.invalidInput
        }
        return try await client.request(method, object)
    }
}
