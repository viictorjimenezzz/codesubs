import Foundation
import Darwin

@main
struct SubscriptionClientChecks {
    static func main() async {
        do {
            let fixture = URL(fileURLWithPath: CommandLine.arguments[1])
            try await testListDecodesSharedFixtureAndDrainsBothStreams(fixture)
            try await testGatewaysDecodeSharedFixtureAndUseApiArguments(URL(fileURLWithPath: CommandLine.arguments[2]))
            try await testMutationsUseExpectedArgumentsAndLoginCodeUsesStdin()
            try await testStructuredFailureAndUnsupportedVersion()
            try await testReconnectStreamsProgressBeforeCompletion()
            try await testCancelledReconnectWaitsForChildCleanup()
            try await testTimeoutTerminatesProcess()
            print("7 Swift CLI client checks passed.")
        } catch {
            FileHandle.standardError.write(Data("Swift client check failed: \(error.localizedDescription.prefix(500))\n".utf8))
            exit(1)
        }
    }

    static func testListDecodesSharedFixtureAndDrainsBothStreams(_ fixture: URL) async throws {
        try checkEqual(
            SubscriptionClient(environment: [:]).executable,
            Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/codesubs")
        )
        let cli = try FakeCLI(script: #"""
        [ "$1" = 'list' ] && [ "$2" = '--json' ] || exit 22
        /bin/dd if=/dev/zero bs=65536 count=4 2>/dev/null | /usr/bin/tr '\000' x >&2
        /bin/cat '\#(fixture.path)' || exit 27
        /bin/dd if=/dev/zero bs=65536 count=4 2>/dev/null | /usr/bin/tr '\000' ' '
        """#)

        let accounts = try await cli.client.list()
        try checkEqual(accounts.map(\.reference), ["claude/work", "codex/personal", "antigravity/team"])
        let claude = try require(accounts.first)
        let observed = try require(claude.lastUpdated)
        try checkEqual(observed.timeIntervalSince1970.truncatingRemainder(dividingBy: 1), 0.125, accuracy: 0.001)
        try checkEqual(claude.home, "/example/vault/claude/work")
        try checkEqual(claude.quota?.weekly?.remainingPercent, 88)
        try checkEqual(claude.quota?.windows.last?.kind, .weeklyModel)
        try checkNil(claude.quota?.windows.last?.resetsAt)
        try checkEqual(claude.quota?.resetCredits?.available, 2)
        try checkEqual(claude.quota?.resetCredits?.grants?.first?.label, "Weekly")
        try checkEqual(claude.quota?.resetCredits?.grants?.last?.paused, true)
        let start = try require(claude.quota?.resetCredits?.grants?.first?.startsAt)
        try checkEqual(start.timeIntervalSince1970.truncatingRemainder(dividingBy: 1), 0.5, accuracy: 0.001)
        try checkEqual(accounts[1].quota?.credits?.balance, 8.5)
        try checkNil(accounts[1].quota?.resetCredits)
        let antigravity = try require(accounts.last)
        try checkEqual(antigravity.provider, .antigravity)
        try checkEqual(antigravity.quota?.windows.map(\.label), ["Gemini Models", "Gemini Models", "Claude and GPT models", "Claude and GPT models"])
        try checkEqual(antigravity.quota?.windows.first?.detail, "Gemini 3.8 Flash")
        try checkEqual(antigravity.quota?.windows.first?.remainingPercent, 97)
        try checkEqual(antigravity.quota?.weekly?.remainingPercent, 80)
        let groups = try require(antigravity.quota?.groups)
        try checkEqual(groups.map(\.name), ["Gemini Models", "Claude and GPT models"])
        try checkEqual(groups.first?.windows.map(\.shortName), ["Week", "5h"])
        try checkEqual(groups.first?.detail, "Gemini 3.8 Flash")
    }

    static func testGatewaysDecodeSharedFixtureAndUseApiArguments(_ fixture: URL) async throws {
        let cli = try FakeCLI(script: #"""
        [ "$1" = api ] || exit 22
        case "$2" in
          list) [ "$#" -eq 3 ] && [ "$3" = --json ] || exit 22; /bin/cat '\#(fixture.path)' ;;
          check) [ "$#" -eq 3 ] && [ "$3" = --json ] || exit 22; /bin/cat '\#(fixture.path)' ;;
          remove) [ "$#" -eq 5 ] && [ "$3" = melious/default ] && [ "$4" = --yes ] && [ "$5" = --json ] || exit 22
            echo '{"protocolVersion":1,"removed":"melious/default"}' ;;
          *) exit 24 ;;
        esac
        """#)

        let gateways = try await cli.client.gateways()
        try checkEqual(gateways.map(\.reference), ["melious/default", "acme/work"])
        let melious = try require(gateways.first)
        try checkEqual(melious.displayName, "Melious")
        try checkEqual(melious.health?.status, .degraded)
        try checkEqual(melious.health?.message, "glm-5.3 is rate limited right now")
        try checkEqual(melious.bigModel, "glm-5.3")
        try checkEqual(melious.smallModel, "glm-5.3-flash")
        try checkNil(gateways.last?.health)
        try checkEqual(gateways.last?.agents, ["codex"])
        try checkEqual(try await cli.client.checkGateways().count, 2)
        try await cli.client.removeGateway("melious/default")
    }

    static func testMutationsUseExpectedArgumentsAndLoginCodeUsesStdin() async throws {
        let cli = try FakeCLI(script: #"""
        case "$1" in
          info)
            [ "$#" -eq 2 ] && [ "$2" = --json ] || exit 22
            echo '{"protocolVersion":1,"version":"test","root":"/vault","guidePath":"/vault/README.md","addGuidePath":"/vault/ADD.md"}' ;;
          refresh)
            [ "$#" -eq 3 ] && [ "$2" = claude/work ] && [ "$3" = --json ] || exit 22
            echo '{"protocolVersion":1,"accounts":[],"failures":[{"reference":"claude/work","code":"provider","message":"Please sign in."}]}' ;;
          rename)
            [ "$#" -eq 5 ] && [ "$2" = --json ] && [ "$3" = claude/work ] && [ "$4" = -- ] && [ "$5" = '--new;name' ] || exit 22
            echo '{"protocolVersion":1}' ;;
          remove)
            [ "$#" -eq 4 ] && [ "$2" = claude/work ] && [ "$3" = --yes ] && [ "$4" = --json ] || exit 22
            echo '{"protocolVersion":1,"removed":"claude/work"}' ;;
          reset)
            [ "$#" -eq 6 ] && [ "$2" = codex/work ] && [ "$3" = --yes ] && [ "$4" = --json ] &&
              [ "$5" = --idempotency-key ] && [ "$6" = '0A07E658-1BDE-4EE0-B802-AE0AEA9B0B7F' ] || exit 22
            echo '{"protocolVersion":1,"outcome":"noCredit","message":"No resets are available."}' ;;
          env)
            [ "$#" -eq 3 ] && [ "$2" = claude/work ] && [ "$3" = --fish ] || exit 22
            printf '%s\n' "set -gx CLAUDE_CONFIG_DIR '/vault/claude/work'" 'set -e ANTHROPIC_API_KEY' ;;
          add-code)
            [ "$#" -eq 5 ] && [ "$2" = --stdin ] && [ "$3" = --slug ] && [ "$4" = work ] && [ "$5" = --json ] || exit 22
            [ "$(/bin/cat)" = 'fake code' ] || exit 23
            echo '{"protocolVersion":1,"sent":true}' ;;
          *) exit 24 ;;
        esac
        """#)

        let info = try await cli.client.info()
        try checkEqual(info.addGuidePath, "/vault/ADD.md")
        let report = try await cli.client.refresh("claude/work")
        try checkEqual(report.failures.first?.message, "Please sign in.")
        try await cli.client.rename("claude/work", to: "--new;name")
        try await cli.client.remove("claude/work")
        let key = try require(UUID(uuidString: "0a07e658-1bde-4ee0-b802-ae0aea9b0b7f"))
        let reset = try await cli.client.reset("codex/work", idempotencyKey: key)
        try checkEqual(reset.outcome, "noCredit")
        let snippet = try await cli.client.environmentSnippet(for: "claude/work", shell: "/opt/bin/fish")
        try checkEqual(snippet, "set -gx CLAUDE_CONFIG_DIR '/vault/claude/work'\nset -e ANTHROPIC_API_KEY")
        try await cli.client.submitCode("fake code", slug: "work")
    }

    static func testStructuredFailureAndUnsupportedVersion() async throws {
        let cli = try FakeCLI(script: #"""
        if [ "$1" = list ]; then
          echo '{"protocolVersion":1,"error":{"code":"corrupt_manifest","message":"Invalid accounts."}}' >&2
          exit 1
        fi
        echo '{"protocolVersion":2,"version":"test","root":"/vault","guidePath":"/vault/README.md","addGuidePath":"/vault/ADD.md"}'
        """#)
        do {
            _ = try await cli.client.list()
            try fail("Expected a structured CLI failure")
        } catch let error as CLIProblem {
            try checkEqual(error.code, "corrupt_manifest")
            try checkEqual(error.localizedDescription, "Invalid accounts.")
        }
        do {
            _ = try await cli.client.info()
            try fail("Expected a version mismatch")
        } catch SubscriptionClientError.unsupportedVersion(let version) {
            try checkEqual(version, 2)
        }
    }

    static func testReconnectStreamsProgressBeforeCompletion() async throws {
        let cli = try FakeCLI { root in
            #"""
            [ "$#" -eq 3 ] && [ "$1" = reconnect ] && [ "$2" = claude/work ] && [ "$3" = --json-events ] || exit 22
            \#(Self.loginEvent)
            for i in $(/usr/bin/jot 100); do
              [ -e '\#(root.path)/release' ] && break
              /bin/sleep 0.02
            done
            [ -e '\#(root.path)/release' ] || exit 25
            echo '{"protocolVersion":1,"type":"waitingForCode","slug":"work","loginID":"test"}'
            printf '%s' '{"protocolVersion":1,"type":"completed","account":{"id":"0a07e658-1bde-4ee0-b802-ae0aea9b0b7f","provider":"claude","slug":"work","reference":"claude/work","home":"/vault/claude/work"}}'
            """#
        }
        let release = cli.root.appendingPathComponent("release")
        try await cli.client.reconnect("claude/work") { instructions in
            guard instructions.loginID == "test", instructions.authURL.host == "example.com",
                  instructions.acceptsCode, !instructions.browserOpened else { return }
            try? Data().write(to: release)
        }
    }

    static func testCancelledReconnectWaitsForChildCleanup() async throws {
        let cli = try FakeCLI { root in
            #"""
            trap 'echo "finishing cleanup"; /bin/sleep 0.1; /usr/bin/touch "\#(root.path)/stopped"; exit 143' TERM
            \#(Self.loginEvent)
            for i in $(/usr/bin/jot 100); do /bin/sleep 0.02; done
            exit 26
            """#
        }
        let (logins, continuation) = AsyncStream<Void>.makeStream()
        let task = Task {
            defer { continuation.finish() }
            try await cli.client.reconnect("claude/work") { _ in continuation.yield(()) }
        }
        var iterator = logins.makeAsyncIterator()
        let received: Void? = await iterator.next()
        try check(received != nil, "Reconnect did not emit a login event before exiting")
        task.cancel()
        do {
            try await task.value
            try fail("Expected cancellation")
        } catch is CancellationError {
            try check(
                FileManager.default.fileExists(atPath: cli.root.appendingPathComponent("stopped").path),
                "Reconnect returned before the CLI finished signal cleanup"
            )
        }
    }

    static func testTimeoutTerminatesProcess() async throws {
        let cli = try FakeCLI(script: "exec /bin/sleep 30", timeout: 0.1)
        do {
            _ = try await cli.client.list()
            try fail("Expected a command timeout")
        } catch SubscriptionClientError.timedOut {
        }
    }

    private static let loginEvent =
        #"echo '{"protocolVersion":1,"type":"login","loginID":"test","authURL":"https://example.com/login","browserOpened":false,"acceptsCode":true,"reference":"claude/work"}'"#
}

private final class FakeCLI {
    let root: URL
    let client: SubscriptionClient

    convenience init(script: String, timeout: TimeInterval = 4) throws {
        try self.init(timeout: timeout) { _ in script }
    }

    init(timeout: TimeInterval = 4, script: (URL) -> String) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        self.root = root
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let executable = root.appendingPathComponent("fake codesubs")
        try ("#!/bin/sh\n" + script(root) + "\n").write(to: executable, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        client = SubscriptionClient(
            environment: ["CODESUBS_CLI": executable.path],
            commandTimeout: timeout
        )
    }

    deinit { try? FileManager.default.removeItem(at: root) }
}

private struct CheckFailure: LocalizedError {
    let detail: String
    var errorDescription: String? { detail }
}

private func fail(_ detail: String, line: UInt = #line) throws -> Never {
    throw CheckFailure(detail: "Line \(line): \(detail)")
}

private func check(_ condition: Bool, _ detail: String, line: UInt = #line) throws {
    guard condition else { try fail(detail, line: line) }
}

private func require<T>(_ value: T?, line: UInt = #line) throws -> T {
    guard let value else { try fail("Expected a value", line: line) }
    return value
}

private func checkEqual<T: Equatable>(_ value: T, _ expected: T, line: UInt = #line) throws {
    try check(value == expected, "Expected \(expected), received \(value)", line: line)
}

private func checkEqual(_ value: Double, _ expected: Double, accuracy: Double, line: UInt = #line) throws {
    try check(abs(value - expected) <= accuracy, "Expected \(expected), received \(value)", line: line)
}

private func checkNil<T>(_ value: T?, line: UInt = #line) throws {
    try check(value == nil, "Expected an absent value", line: line)
}
