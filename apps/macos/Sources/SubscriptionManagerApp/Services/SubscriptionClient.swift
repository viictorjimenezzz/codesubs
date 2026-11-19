import Foundation

struct SubscriptionClient: Sendable {
    let executable: URL
    private let commandTimeout: TimeInterval

    init(
        bundle: Bundle = .main,
        environment: [String: String] = ProcessInfo.processInfo.environment,
        commandTimeout: TimeInterval = 15
    ) {
        if let override = environment["SUBSCRIPTION_MANAGER_CLI"], !override.isEmpty {
            executable = URL(fileURLWithPath: override)
        } else {
            executable = bundle.bundleURL.appendingPathComponent("Contents/Helpers/subscription-manager")
        }
        self.commandTimeout = commandTimeout
    }

    func list() async throws -> [AccountRecord] {
        let data = try await output(["list", "--json"])
        return try Self.decode([AccountRecord].self, data)
    }

    func info() async throws -> ClientInfo {
        try await response(ClientInfo.self, arguments: ["info", "--json"])
    }

    func refresh(_ account: String? = nil, timeout: TimeInterval = 5 * 60) async throws -> RefreshReport {
        try await response(
            RefreshReport.self,
            arguments: ["refresh"] + (account.map { [$0] } ?? []) + ["--json"],
            timeout: timeout
        )
    }

    func rename(_ account: String, to slug: String) async throws {
        // Stop option parsing before passing the verbatim editor input.
        _ = try await output(["rename", "--json", account, "--", slug], timeout: 90)
    }

    func remove(_ account: String) async throws {
        _ = try await output(["remove", account, "--yes", "--json"], timeout: 90)
    }

    func reset(_ account: String, idempotencyKey: UUID) async throws -> ResetReport {
        try await response(
            ResetReport.self,
            arguments: ["reset", account, "--yes", "--json", "--idempotency-key", idempotencyKey.uuidString],
            timeout: 5 * 60
        )
    }

    func environmentSnippet(
        for account: String,
        shell: String? = ProcessInfo.processInfo.environment["SHELL"]
    ) async throws -> String {
        let flag = shell.map { ($0 as NSString).lastPathComponent } == "fish" ? "--fish" : "--sh"
        let data = try await output(["env", account, flag])
        return String(decoding: data, as: UTF8.self).trimmingCharacters(in: .newlines)
    }

    func reconnect(
        _ account: String,
        onLogin: @Sendable (LoginInstructions) async -> Void
    ) async throws {
        let (events, continuation) = AsyncStream<LoginEvent>.makeStream()
        async let result: Void = collectLoginEvents(account, into: continuation)
        var completed = false
        for await event in events {
            switch event {
            case .login(let instructions): await onLogin(instructions)
            case .waitingForCode: break
            case .completed: completed = true
            case .error(let error): throw error
            }
        }
        try Task.checkCancellation()
        try await result
        guard completed else { throw SubscriptionClientError.invalidResponse }
    }

    private func collectLoginEvents(
        _ account: String,
        into continuation: AsyncStream<LoginEvent>.Continuation
    ) async throws {
        defer { continuation.finish() }
        _ = try await output(
            ["reconnect", account, "--json-events"],
            timeout: 12 * 60,
            onLine: { line in continuation.yield(try Self.decode(LoginEvent.self, line)) }
        )
    }

    func submitCode(_ code: String, slug: String) async throws {
        _ = try await output(["add-code", "--stdin", "--slug", slug, "--json"], input: Data(code.utf8))
    }

    private func response<T: VersionedResponse>(
        _ type: T.Type,
        arguments: [String],
        timeout: TimeInterval? = nil
    ) async throws -> T {
        let value = try Self.decode(type, await output(arguments, timeout: timeout))
        try SubscriptionClientError.checkVersion(value.protocolVersion)
        return value
    }

    private func output(
        _ arguments: [String],
        input: Data? = nil,
        timeout: TimeInterval? = nil,
        onLine: (@Sendable (Data) throws -> Void)? = nil
    ) async throws -> Data {
        let result = try await CLIProcess(
            executable: executable,
            arguments: arguments,
            input: input,
            timeout: timeout ?? commandTimeout,
            onLine: onLine
        ).run()
        try Task.checkCancellation()

        if result.status != 0 {
            struct ErrorEnvelope: Decodable {
                let error: CLIProblem
            }
            let decoder = JSONDecoder()
            if let envelope = try? decoder.decode(ErrorEnvelope.self, from: result.stderr) {
                throw envelope.error
            }
            if let problem = try? decoder.decode(CLIProblem.self, from: result.stderr) {
                throw problem
            }
            let detail = String(decoding: result.stderr, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            throw SubscriptionClientError.commandFailed(result.status, detail)
        }
        return result.stdout
    }

    private static func decode<T: Decodable>(_ type: T.Type, _ data: Data) throws -> T {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let value = try decoder.singleValueContainer().decode(String.self)
            let formatter = ISO8601DateFormatter()
            formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            if let date = formatter.date(from: value) { return date }
            formatter.formatOptions = [.withInternetDateTime]
            if let date = formatter.date(from: value) { return date }
            throw SubscriptionClientError.invalidResponse
        }
        do {
            return try decoder.decode(type, from: data)
        } catch let error as SubscriptionClientError {
            throw error
        } catch {
            throw SubscriptionClientError.invalidResponse
        }
    }
}

enum SubscriptionClientError: LocalizedError {
    case launchFailed(String)
    case timedOut
    case commandFailed(Int32, String)
    case invalidResponse
    case unsupportedVersion(Int)

    static func checkVersion(_ version: Int) throws {
        guard version == 1 else { throw Self.unsupportedVersion(version) }
    }

    var errorDescription: String? {
        switch self {
        case .launchFailed(let reason): return "Could not start subscription-manager. \(reason)"
        case .timedOut: return "The subscription-manager command timed out."
        case .commandFailed(let status, let detail):
            return detail.isEmpty ? "Subscription Manager CLI exited with status \(status)." : detail
        case .invalidResponse: return "Subscription Manager CLI returned an invalid response."
        case .unsupportedVersion(let version): return "Subscription Manager CLI returned unsupported protocol version \(version)."
        }
    }
}
