import Foundation

enum Provider: String, Codable, CaseIterable, Sendable, Identifiable {
    case codex
    case claude
    case antigravity

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .codex: return "Codex"
        case .claude: return "Claude Code"
        case .antigravity: return "Antigravity"
        }
    }

    var logoAsset: String {
        switch self {
        case .codex: return "openai-logo.png"
        case .claude: return "claude-color.png"
        case .antigravity: return "antigravity-icon.png"
        }
    }

    var logoIsTemplate: Bool { self == .codex }
}

struct AccountRecord: Codable, Identifiable, Sendable, Equatable {
    let id: UUID
    let provider: Provider
    let slug: String
    let reference: String
    let home: String
    let email: String?
    let plan: String?
    let quota: QuotaSnapshot?
    let lastUpdated: Date?
}

struct QuotaWindow: Codable, Sendable, Equatable, Identifiable {
    enum Kind: String, Codable, Sendable {
        case session
        case weekly
        case weeklyModel
    }

    let kind: Kind
    let label: String
    let usedPercent: Int
    let resetsAt: Date?
    let windowDurationMinutes: Int?
    /// What the window covers, such as the models sharing a group limit.
    let detail: String?

    var id: String { "\(kind.rawValue)-\(label)-\(windowDurationMinutes ?? 0)" }
    var remainingPercent: Int { 100 - min(max(usedPercent, 0), 100) }

    /// The window's length, such as "Week" or "5h".
    var shortName: String {
        switch windowDurationMinutes {
        case 10080: return "Week"
        case 1440: return "Day"
        case let minutes? where minutes > 0 && minutes % 60 == 0: return "\(minutes / 60)h"
        default: return kind == .session ? "Session" : "Week"
        }
    }
}

/// Models that share one set of limits, such as Antigravity's "Gemini Models".
struct QuotaGroup: Identifiable, Equatable {
    let name: String
    let detail: String?
    let windows: [QuotaWindow]

    var id: String { name }
}

struct ResetCredits: Codable, Sendable, Equatable {
    let available: Int
    let nextExpiry: Date?
    let grants: [ResetGrant]?
}

struct ResetGrant: Codable, Sendable, Equatable {
    let label: String
    let available: Int
    let startsAt: Date?
    let expiresAt: Date?
    let paused: Bool

    func isDisplayed(at date: Date) -> Bool {
        available > 0 && !paused && (startsAt ?? .distantPast) <= date && (expiresAt ?? .distantFuture) > date
    }
}

struct CreditBalance: Codable, Sendable, Equatable {
    let balance: Double?
    let unlimited: Bool
}

struct QuotaSnapshot: Codable, Sendable, Equatable {
    let windows: [QuotaWindow]
    let resetCredits: ResetCredits?
    let credits: CreditBalance?

    /// The quota window drawn as the headline bar: the weekly window, or the
    /// tightest one when several model groups each have their own.
    var weekly: QuotaWindow? {
        windows.filter { $0.kind == .weekly }.min(by: { $0.remainingPercent < $1.remainingPercent })
            ?? windows.max(by: { ($0.windowDurationMinutes ?? 0) < ($1.windowDurationMinutes ?? 0) })
    }

    /// Windows grouped by the models they cover, in the order received,
    /// longest window first within each group.
    var groups: [QuotaGroup] {
        var names: [String] = []
        var byName: [String: [QuotaWindow]] = [:]
        for window in windows {
            if byName[window.label] == nil { names.append(window.label) }
            byName[window.label, default: []].append(window)
        }
        return names.map { name in
            let windows = byName[name, default: []]
                .sorted { ($0.windowDurationMinutes ?? 0) > ($1.windowDurationMinutes ?? 0) }
            return QuotaGroup(name: name, detail: windows.first?.detail, windows: windows)
        }
    }
}

enum GatewayStatus: String, Codable, Sendable {
    case ok
    case degraded
    case down
}

struct GatewayHealth: Codable, Sendable, Equatable {
    let status: GatewayStatus
    let checkedAt: Date
    let latencyMs: Int
    let message: String?
}

/// An API-key account, such as Melious, that runs Claude Code or Codex.
struct GatewayRecord: Codable, Identifiable, Sendable, Equatable {
    let id: UUID
    let service: String
    let slug: String
    let reference: String
    let agents: [String]
    let bigModel: String
    let smallModel: String
    let health: GatewayHealth?

    var displayName: String { service.prefix(1).uppercased() + service.dropFirst() }
}

struct GatewayReport: VersionedResponse, Sendable {
    let protocolVersion: Int
    let gateways: [GatewayRecord]
}

protocol VersionedResponse: Decodable {
    var protocolVersion: Int { get }
}

struct ClientInfo: VersionedResponse, Sendable {
    let protocolVersion: Int
    let root: String
    let guidePath: String
    let addGuidePath: String
    let version: String
}

struct RefreshReport: VersionedResponse, Sendable {
    struct Failure: Decodable, Sendable {
        let reference: String
        let code: String
        let message: String
    }

    let protocolVersion: Int
    let accounts: [AccountRecord]
    let failures: [Failure]
}

struct ResetReport: VersionedResponse, Sendable {
    let protocolVersion: Int
    let outcome: String
    let message: String
    let account: AccountRecord?
}

struct LoginInstructions: Decodable, Sendable, Equatable {
    let loginID: String
    let authURL: URL
    let browserOpened: Bool
    let acceptsCode: Bool
    let reference: String
}

enum LoginEvent: Decodable, Sendable {
    case login(LoginInstructions)
    case waitingForCode
    case completed(AccountRecord)
    case error(CLIProblem)

    private enum CodingKeys: String, CodingKey { case protocolVersion, type, account }
    private enum Kind: String, Decodable { case login, waitingForCode, completed, error }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        try SubscriptionClientError.checkVersion(values.decode(Int.self, forKey: .protocolVersion))
        switch try values.decode(Kind.self, forKey: .type) {
        case .login: self = .login(try LoginInstructions(from: decoder))
        case .waitingForCode: self = .waitingForCode
        case .completed: self = .completed(try values.decode(AccountRecord.self, forKey: .account))
        case .error: self = .error(try CLIProblem(from: decoder))
        }
    }
}

struct CLIProblem: Decodable, LocalizedError, Sendable {
    let code: String
    let message: String

    var errorDescription: String? { message }
}

enum AccountRuntimeState: Equatable {
    case idle
    case connecting
    case awaitingLogin
    case refreshing
    case ready
    case failed(String)

    var isBusy: Bool {
        self == .connecting || self == .awaitingLogin || self == .refreshing
    }
}
