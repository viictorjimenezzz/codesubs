import AppKit
import Combine
import Foundation

@MainActor
final class AccountViewModel: ObservableObject {
    private static let resetRequestPrefix = "pendingResetRequest."

    @Published private(set) var accounts: [AccountRecord] = []
    @Published private(set) var states: [UUID: AccountRuntimeState] = [:]
    @Published private(set) var info: ClientInfo?
    @Published private(set) var activeLogin: LoginInstructions?
    @Published private(set) var isSigningIn = false
    @Published private(set) var isRefreshingAll = false
    @Published private var syncError: String?
    @Published private var infoError: String?
    @Published var globalError: String?
    @Published var notice: String?

    private let client: SubscriptionClient
    private var refreshTask: Task<Void, Never>?
    private var loginTask: Task<Void, Never>?
    private var isLoadingInfo = false
    private var reloadRequest = 0
    private var lastAppliedReload = 0

    init(client: SubscriptionClient = SubscriptionClient()) {
        self.client = client
        Self.migratePreviousResetRequests()
        refreshTask = Task { [weak self] in
            do {
                try await Task.sleep(for: .seconds(1))
                while !Task.isCancelled {
                    await self?.refreshAll()
                    try await Task.sleep(for: .seconds(15 * 60))
                }
            } catch {}
        }
    }

    deinit {
        refreshTask?.cancel()
        loginTask?.cancel()
    }

    var errorMessage: String? { globalError ?? syncError ?? infoError }

    func accounts(for provider: Provider) -> [AccountRecord] {
        accounts.filter { $0.provider == provider }
    }

    func pollWhileVisible() async {
        do {
            while !Task.isCancelled {
                await reload()
                await loadInfo()
                try await Task.sleep(for: .seconds(5))
            }
        } catch {}
    }

    func refreshAll() async {
        guard !isRefreshingAll, !states.values.contains(where: \.isBusy) else { return }
        globalError = nil
        isRefreshingAll = true
        defer { isRefreshingAll = false }

        await reload()
        guard !Task.isCancelled, !accounts.isEmpty, !states.values.contains(where: \.isBusy) else { return }
        let targets = accounts
        for account in targets { states[account.id] = .refreshing }
        defer {
            for account in targets { finishWorking(account.id) }
        }

        do {
            let timeout = max(5 * 60, TimeInterval(targets.count) * 150 + 60)
            let report = try await client.refresh(timeout: timeout)
            await reload()
            for failure in report.failures {
                if let account = targets.first(where: { $0.reference == failure.reference }),
                   accounts.contains(where: { $0.id == account.id && $0.reference == failure.reference }) {
                    states[account.id] = .failed(failure.message)
                }
            }
        } catch is CancellationError {
        } catch {
            globalError = error.localizedDescription
        }
    }

    func rename(_ id: UUID, to slug: String) async throws {
        guard let account = account(id), states[id]?.isBusy != true, slug != account.slug else { return }
        states[id] = .refreshing
        defer { finishWorking(id) }
        try await client.rename(account.reference, to: slug)
        await reload()
    }

    func remove(_ id: UUID) async {
        guard let account = account(id), states[id]?.isBusy != true else { return }
        globalError = nil
        states[id] = .refreshing
        defer { finishWorking(id) }
        do {
            try await client.remove(account.reference)
            UserDefaults.standard.removeObject(forKey: resetRequestKey(id))
            await reload()
        } catch {
            globalError = error.localizedDescription
        }
    }

    func applyReset(_ id: UUID) async {
        guard let account = account(id), states[id]?.isBusy != true else { return }
        globalError = nil
        states[id] = .refreshing
        defer { finishWorking(id) }
        notice = nil
        let key = pendingResetRequest(id) ?? UUID()
        UserDefaults.standard.set(key.uuidString, forKey: resetRequestKey(id))
        do {
            let report = try await client.reset(account.reference, idempotencyKey: key)
            notice = "\(account.reference): \(report.message)"
            UserDefaults.standard.removeObject(forKey: resetRequestKey(id))
            await reload()
        } catch {
            globalError = error.localizedDescription
        }
    }

    func hasPendingReset(_ id: UUID) -> Bool {
        pendingResetRequest(id) != nil
    }

    private func pendingResetRequest(_ id: UUID) -> UUID? {
        UserDefaults.standard.string(forKey: resetRequestKey(id)).flatMap(UUID.init(uuidString:))
    }

    private func resetRequestKey(_ id: UUID) -> String {
        "\(Self.resetRequestPrefix)\(id.uuidString)"
    }

    private static func migratePreviousResetRequests() {
        guard Bundle.main.bundleIdentifier == "com.viictorjimenezzz.codesubs" else { return }
        let defaults = UserDefaults.standard
        let migratedKey = "migratedSubscriptionManagerResetRequests"
        guard !defaults.bool(forKey: migratedKey) else { return }
        let previous = defaults.persistentDomain(forName: "com.viictorjimenezzz.subscription-manager") ?? [:]
        for (key, value) in previous where key.hasPrefix(resetRequestPrefix) {
            if defaults.object(forKey: key) == nil, let token = value as? String, UUID(uuidString: token) != nil {
                defaults.set(token, forKey: key)
            }
        }
        defaults.set(true, forKey: migratedKey)
    }

    func environmentSnippet(for account: AccountRecord) async -> String? {
        do {
            return try await client.environmentSnippet(for: account.reference)
        } catch {
            globalError = error.localizedDescription
            return nil
        }
    }

    func reconnect(_ id: UUID) {
        guard let account = account(id), loginTask == nil, states[id]?.isBusy != true else { return }
        globalError = nil
        isSigningIn = true
        states[id] = .connecting
        loginTask = Task { [weak self] in
            await self?.runReconnect(account)
        }
    }

    private func runReconnect(_ account: AccountRecord) async {
        defer {
            activeLogin = nil
            isSigningIn = false
            loginTask = nil
            finishWorking(account.id, if: .connecting)
            finishWorking(account.id, if: .awaitingLogin)
        }

        do {
            try await client.reconnect(account.reference) { [weak self] instructions in
                await self?.presentLogin(instructions, for: account.id)
            }
            await reload()
        } catch is CancellationError {
            states[account.id] = .failed("Login cancelled.")
        } catch {
            states[account.id] = .failed(error.localizedDescription)
            await reload()
        }
    }

    private func presentLogin(_ instructions: LoginInstructions, for id: UUID) {
        guard !Task.isCancelled else { return }
        activeLogin = instructions
        states[id] = .awaitingLogin
        if !instructions.browserOpened { NSWorkspace.shared.open(instructions.authURL) }
    }

    func submitLoginCode(_ code: String) async -> Bool {
        guard let instructions = activeLogin, instructions.acceptsCode else { return false }
        let slug = (instructions.reference as NSString).lastPathComponent
        globalError = nil
        do {
            try await client.submitCode(code, slug: slug)
            return true
        } catch {
            globalError = error.localizedDescription
            return false
        }
    }

    func cancelActiveLogin() async {
        let task = loginTask
        task?.cancel()
        await task?.value
    }

    func shutDown() async {
        refreshTask?.cancel()
        await cancelActiveLogin()
        await refreshTask?.value
    }

    private func account(_ id: UUID) -> AccountRecord? {
        accounts.first(where: { $0.id == id })
    }

    private func finishWorking(_ id: UUID, if state: AccountRuntimeState = .refreshing) {
        guard states[id] == state else { return }
        states[id] = account(id)?.quota == nil ? .idle : .ready
    }

    private func reload() async {
        reloadRequest += 1
        let request = reloadRequest
        do {
            let result = try await client.list()
            try Task.checkCancellation()
            guard request >= lastAppliedReload else { return }
            lastAppliedReload = request
            let previous = Dictionary(accounts.map { ($0.id, $0.lastUpdated ?? .distantPast) }, uniquingKeysWith: { _, last in last })
            accounts = result
            states = Dictionary(result.map { account in
                let state = states[account.id]
                let observed = account.lastUpdated ?? .distantPast
                let ready: AccountRuntimeState = account.quota == nil ? .idle : .ready
                if let state, state.isBusy || observed <= (previous[account.id] ?? .distantPast) {
                    return (account.id, state)
                }
                return (account.id, ready)
            }, uniquingKeysWith: { _, last in last })
            syncError = nil
        } catch is CancellationError {
        } catch {
            guard request >= lastAppliedReload else { return }
            lastAppliedReload = request
            syncError = error.localizedDescription
        }
    }

    private func loadInfo() async {
        guard info == nil, !isLoadingInfo else { return }
        isLoadingInfo = true
        defer { isLoadingInfo = false }
        do {
            info = try await client.info()
            infoError = nil
        } catch is CancellationError {
        } catch {
            infoError = error.localizedDescription
        }
    }
}
