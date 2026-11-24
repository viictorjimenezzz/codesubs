import AppKit
import SwiftUI

struct QuotaPopoverView: View {
    @ObservedObject var store: AccountViewModel
    @AppStorage("launchAtLogin") private var launchAtLogin = false

    var body: some View {
        ZStack {
            TahoeTranslucentBackground().ignoresSafeArea()
            Color.primary.opacity(0.018).ignoresSafeArea()
            VStack(spacing: 0) {
                header
                Divider().opacity(0.32)
                content
                Divider().opacity(0.32)
                footer
            }
        }
        .frame(width: 364)
        .fixedSize(horizontal: false, vertical: true)
        .preferredColorScheme(nil)
        .task { await store.pollWhileVisible() }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Text("Codesubs")
                .font(.system(size: 13.5, weight: .semibold, design: .rounded))
            Spacer()
            Button {
                Task { await store.refreshAll() }
            } label: {
                Image(systemName: "arrow.clockwise")
                    .rotationEffect(store.isRefreshingAll ? .degrees(360) : .zero)
                    .animation(store.isRefreshingAll ? .linear(duration: 0.8).repeatForever(autoreverses: false) : .default, value: store.isRefreshingAll)
            }
            .buttonStyle(.plain)
            .help("Refresh all accounts")
            .disabled((store.accounts.isEmpty && store.gateways.isEmpty) || store.isRefreshingAll || store.states.values.contains(where: \.isBusy))
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    @ViewBuilder
    private var content: some View {
        if store.accounts.isEmpty && store.gateways.isEmpty {
            VStack(spacing: 13) {
                Image(systemName: "gauge.with.dots.needle.33percent")
                    .font(.system(size: 28, weight: .light))
                    .foregroundStyle(.secondary)
                Text("No subscriptions yet.")
                    .font(.system(size: 13, weight: .medium))
                Text("Ask your agent to add one. It follows ADD.md in the credentials folder.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                Button("Open ADD.md", action: openAddGuide)
                    .controlSize(.small)
                    .disabled(store.info == nil)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 28)
        } else {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(Provider.allCases) { provider in
                    let accounts = store.accounts(for: provider)
                    if !accounts.isEmpty {
                        ProviderHeader(provider: provider)
                        ForEach(accounts) { account in
                            AccountRow(
                                store: store,
                                account: account,
                                state: store.states[account.id] ?? .idle
                            )
                        }
                    }
                }
                if !store.gateways.isEmpty {
                    GatewayHeader()
                    ForEach(store.gateways) { gateway in
                        GatewayRow(store: store, gateway: gateway)
                    }
                }
            }
            .padding(.vertical, 4)
        }

        if let login = store.activeLogin {
            Divider().opacity(0.5)
            LoginBanner(
                instructions: login,
                onSubmitCode: store.submitLoginCode,
                onCancel: { Task { await store.cancelActiveLogin() } }
            )
            .id(login.loginID)
        } else if store.isSigningIn {
            Divider().opacity(0.5)
            HStack {
                ProgressView().controlSize(.small)
                Text("Starting sign-in")
                Spacer()
                Button("Cancel") { Task { await store.cancelActiveLogin() } }
                    .buttonStyle(.plain)
            }
            .font(.caption)
            .padding(13)
        }

        if let notice = store.notice {
            HStack(alignment: .firstTextBaseline) {
                Text(notice)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
                Button("OK") { store.notice = nil }
                    .buttonStyle(.link)
                    .font(.caption)
            }
            .padding(12)
        }

        if let error = store.errorMessage {
            Text(error)
                .font(.caption)
                .foregroundStyle(.red)
                .fixedSize(horizontal: false, vertical: true)
                .padding(12)
        }
    }

    private var footer: some View {
        HStack(spacing: 12) {
            Button(action: openAddGuide) {
                Label("Adding accounts", systemImage: "questionmark.circle")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .disabled(store.info == nil)
            .help("New subscriptions are added by an agent following ADD.md")

            Spacer()
            Menu {
                Toggle("Launch at login", isOn: Binding(
                    get: { launchAtLogin },
                    set: { value in
                        do {
                            try LaunchAtLogin.setEnabled(value)
                            launchAtLogin = value
                        } catch {
                            store.globalError = error.localizedDescription
                        }
                    }
                ))
                Divider()
                Button("Open credentials folder") {
                    if let info = store.info { NSWorkspace.shared.open(URL(fileURLWithPath: info.root)) }
                }
                .disabled(store.info == nil)
                Button("Copy credentials folder path") {
                    if let info = store.info { copyToPasteboard(info.root) }
                }
                .disabled(store.info == nil)
                Divider()
                Button("Quit Codesubs") { NSApplication.shared.terminate(nil) }
            } label: {
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
        }
        .font(.caption)
        .padding(.horizontal, 16)
        .padding(.vertical, 11)
    }

    private func openAddGuide() {
        if let info = store.info { NSWorkspace.shared.open(URL(fileURLWithPath: info.addGuidePath)) }
    }
}

private struct ProviderHeader: View {
    let provider: Provider

    var body: some View {
        HStack(spacing: 6) {
            ProviderLogo(provider: provider)
                .frame(width: 12, height: 12)
            Text(provider.displayName.uppercased())
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(.secondary)
                .tracking(0.4)
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
        .padding(.bottom, 1)
    }
}

private struct GatewayHeader: View {
    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "key.horizontal")
                .font(.system(size: 9.5))
                .foregroundStyle(.secondary)
                .frame(width: 12, height: 12)
            Text("API KEYS")
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(.secondary)
                .tracking(0.4)
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
        .padding(.bottom, 1)
    }
}

/// One API-key account: a dot that is green only while its key is accepted
/// and both its models answer, followed by the model pair it launches with.
private struct GatewayRow: View {
    @ObservedObject var store: AccountViewModel
    let gateway: GatewayRecord
    @State private var confirmingRemoval = false

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                statusDot
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    Text(gateway.displayName)
                        .font(.system(size: 12, weight: .medium))
                    if gateway.slug != "default" {
                        Text("· \(gateway.slug)")
                            .font(.system(size: 10.5))
                            .foregroundStyle(.tertiary)
                    }
                }
                .lineLimit(1)
                Spacer(minLength: 6)
                if store.isCheckingGateways {
                    ProgressView().controlSize(.small).scaleEffect(0.7)
                } else {
                    Text(statusText)
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(statusColor)
                }
                menu
            }
            Text("\(gateway.bigModel) · \(gateway.smallModel)")
                .font(.system(size: 10.5, design: .monospaced))
                .foregroundStyle(.tertiary)
                .lineLimit(1)
                .truncationMode(.middle)
            if let message = gateway.health?.message {
                Text(message)
                    .font(.caption2)
                    .foregroundStyle(statusColor)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if confirmingRemoval {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Remove \(gateway.reference) and its stored key?")
                        .font(.caption)
                    HStack(spacing: 8) {
                        Button("Remove", role: .destructive) {
                            confirmingRemoval = false
                            Task { await store.removeGateway(gateway.id) }
                        }
                        .controlSize(.small)
                        .buttonStyle(.bordered)
                        Button("Cancel") { confirmingRemoval = false }
                            .controlSize(.small)
                            .buttonStyle(.plain)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(8)
                .background(RoundedRectangle(cornerRadius: 6).fill(.secondary.opacity(0.08)))
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .help(helpText)
    }

    private var statusDot: some View {
        Circle()
            .fill(statusColor)
            .frame(width: 8, height: 8)
            .accessibilityLabel(statusText)
    }

    private var statusText: String {
        switch gateway.health?.status {
        case .ok: return "Operative"
        case .degraded: return "Degraded"
        case .down: return "Down"
        case nil: return "Not checked"
        }
    }

    private var statusColor: Color {
        switch gateway.health?.status {
        case .ok: return color(forRemaining: 100)
        case .degraded: return color(forRemaining: 30)
        case .down: return color(forRemaining: 0)
        case nil: return .gray
        }
    }

    private var helpText: String {
        guard let health = gateway.health else { return "Not checked yet" }
        return "Checked \(health.checkedAt.formatted(.relative(presentation: .named))) · \(health.latencyMs) ms"
    }

    private var menu: some View {
        Menu {
            Button("Check now") { Task { await store.checkGateways() } }
                .disabled(store.isCheckingGateways)
            Divider()
            ForEach(gateway.agents, id: \.self) { agent in
                Button("Copy \(agent == "claude" ? "Claude Code" : "Codex") launch command") {
                    copyToPasteboard("codesubs exec \(gateway.reference) -- \(agent)")
                }
            }
            Divider()
            Button("Remove API account…", role: .destructive) { confirmingRemoval = true }
        } label: {
            Image(systemName: "ellipsis")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 18, height: 16)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
    }
}

private struct ProviderLogo: View {
    let provider: Provider

    var body: some View {
        if let url = Bundle.main.resourceURL?.appendingPathComponent("Assets/\(provider.logoAsset)"),
           let image = NSImage(contentsOf: url) {
            Image(nsImage: image)
                .renderingMode(provider.logoIsTemplate ? .template : .original)
                .resizable()
                .scaledToFit()
                .foregroundStyle(.primary)
        } else {
            Image(systemName: provider == .codex ? "terminal" : "asterisk").resizable().scaledToFit()
        }
    }
}

private struct AccountRow: View {
    @ObservedObject var store: AccountViewModel
    let account: AccountRecord
    let state: AccountRuntimeState

    @State private var editingSlug: String?
    @State private var renameError: String?
    @State private var confirmingReset = false
    @State private var confirmingRemoval = false

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 8) {
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    Text(account.reference)
                        .font(.system(size: 11.5, weight: .medium, design: .monospaced))
                        .truncationMode(.middle)
                    if let plan = account.plan {
                        Text("· \(plan)")
                            .font(.system(size: 10.5))
                            .foregroundStyle(.tertiary)
                            .fixedSize()
                    }
                }
                .lineLimit(1)
                Spacer(minLength: 6)
                if state.isBusy {
                    ProgressView().controlSize(.small).scaleEffect(0.7)
                } else if let weekly = account.quota?.weekly {
                    Text("\(weekly.remainingPercent)%")
                        .font(.system(size: 11.5, weight: .semibold, design: .rounded).monospacedDigit())
                        .foregroundStyle(color(forRemaining: weekly.remainingPercent))
                }
                rowMenu
            }

            if editingSlug != nil {
                renameEditor
            }

            if confirmingReset {
                resetConfirmation
            }

            if confirmingRemoval {
                removeConfirmation
            }

            if let quota = account.quota, let weekly = quota.weekly {
                QuotaBar(remaining: weekly.remainingPercent, color: color(forRemaining: weekly.remainingPercent))

                HStack(spacing: 10) {
                    HStack(spacing: 4) {
                        Text(resetText(for: weekly))
                        if let credits = quota.resetCredits, bankedResets > 0 {
                            Text("· \(bankedResets) \(bankedResets == 1 ? "reset" : "resets")")
                                .help(resetHelp(credits))
                        } else if account.provider == .claude, quota.resetCredits == nil {
                            Text("· Resets unavailable")
                                .help("Could not read the reset balance. Refresh or check Claude Settings > Usage.")
                        }
                    }
                    .lineLimit(1)
                    Spacer(minLength: 4)
                    if let credits = quota.credits {
                        Text("Credits \(creditText(credits))")
                            .help("ChatGPT credits balance")
                    }
                    ForEach(secondaryWindows(quota)) { window in
                        Text("\(window.label) \(window.remainingPercent)%")
                            .foregroundStyle(window.remainingPercent < 25 ? AnyShapeStyle(color(forRemaining: window.remainingPercent)) : AnyShapeStyle(.tertiary))
                            .help(resetText(for: window))
                    }
                }
                .font(.system(size: 10.5).monospacedDigit())
                .foregroundStyle(.tertiary)

                if case .failed(let message) = state {
                    failure(message + (account.lastUpdated.map { " The numbers above are from \($0.formatted(.relative(presentation: .named)))." } ?? ""))
                }
            } else if case .failed(let message) = state {
                failure(message)
            } else {
                Text(state == .awaitingLogin ? "Finish signing in in your browser" : "Waiting for quota data")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 9)
        .help(account.lastUpdated.map { "Updated \($0.formatted(.relative(presentation: .named)))" } ?? "")
    }

    private func failure(_ message: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(message)
                .font(.caption2)
                .foregroundStyle(.red)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            Button("Sign in") { store.reconnect(account.id) }
                .buttonStyle(.link)
                .font(.caption2)
                .disabled(store.isSigningIn)
        }
    }

    private var bankedResets: Int { account.quota?.resetCredits?.available ?? 0 }
    private var hasPendingReset: Bool { store.hasPendingReset(account.id) }

    private var resetMenuTitle: String {
        if hasPendingReset { return "Retry reset…" }
        if bankedResets > 0 { return "Apply reset… (\(bankedResets) banked)" }
        return "No banked resets"
    }

    private func resetHelp(_ credits: ResetCredits) -> String {
        let action = account.provider == .codex
            ? "Banked resets. Apply one from the ⋯ menu."
            : "Banked Claude resets. See each grant in the ⋯ menu; redeem in Claude Settings > Usage."
        return action + (credits.nextExpiry.map {
            " The next one expires \($0.formatted(date: .abbreviated, time: .shortened))."
        } ?? "")
    }

    private func creditText(_ credits: CreditBalance) -> String {
        if credits.unlimited { return "∞" }
        guard let balance = credits.balance else { return "–" }
        return balance.formatted(.number.precision(.fractionLength(0)))
    }

    private var rowMenu: some View {
        Menu {
            Button("Copy environment for agents") {
                Task {
                    if let snippet = await store.environmentSnippet(for: account) { copyToPasteboard(snippet) }
                }
            }
            Button("Copy folder path") { copyToPasteboard(account.home) }
            Button("Show folder in Finder") {
                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: account.home)])
            }
            Button("Rename folder…") {
                renameError = nil
                editingSlug = account.slug
            }
            .disabled(state.isBusy)
            if account.provider == .codex {
                Divider()
                Button(resetMenuTitle) {
                    confirmingReset = true
                }
                .disabled((bankedResets == 0 && !hasPendingReset) || state.isBusy)
            } else {
                Divider()
                Text(account.quota?.resetCredits == nil ? "Reset balance unavailable" : "\(bankedResets) banked \(bankedResets == 1 ? "reset" : "resets")")
                if let grants = account.quota?.resetCredits?.grants {
                    ForEach(Array(grants.filter { $0.isDisplayed(at: Date()) }.enumerated()), id: \.offset) { _, grant in
                        Text("\(grant.available) · \(grant.label)")
                        if let expiry = grant.expiresAt {
                            Text("Expires \(expiry.formatted(date: .abbreviated, time: .shortened))")
                        }
                    }
                }
                Button("Open Claude usage…") {
                    NSWorkspace.shared.open(URL(string: "https://claude.ai/settings/usage")!)
                }
                .help("Use the browser account \(account.email ?? account.reference) to view or redeem its resets.")
            }
            Divider()
            Button("Sign in again") { store.reconnect(account.id) }
                .disabled(store.isSigningIn || state.isBusy)
            Button("Remove account…", role: .destructive) { confirmingRemoval = true }
                .disabled(state.isBusy)
        } label: {
            Image(systemName: "ellipsis")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 18, height: 16)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
    }

    private var resetConfirmation: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(hasPendingReset
                 ? "Retry the previous reset request for this account?"
                 : "Use 1 of \(bankedResets) banked resets? Codex resets this account's current limits now. If nothing is eligible, no credit is used.")
                .font(.caption)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 8) {
                Button(hasPendingReset ? "Retry reset" : "Apply reset") {
                    confirmingReset = false
                    Task { await store.applyReset(account.id) }
                }
                .controlSize(.small)
                .buttonStyle(.borderedProminent)
                .disabled(state.isBusy)
                Button("Cancel") { confirmingReset = false }
                    .controlSize(.small)
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(8)
        .background(RoundedRectangle(cornerRadius: 6).fill(.secondary.opacity(0.08)))
    }

    private var removeConfirmation: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Remove \(account.reference) and its local credentials?")
                .font(.caption)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 8) {
                Button("Remove account", role: .destructive) {
                    confirmingRemoval = false
                    Task { await store.remove(account.id) }
                }
                .controlSize(.small)
                .buttonStyle(.bordered)
                .disabled(state.isBusy)
                Button("Cancel") { confirmingRemoval = false }
                    .controlSize(.small)
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(8)
        .background(RoundedRectangle(cornerRadius: 6).fill(.secondary.opacity(0.08)))
    }

    private var renameEditor: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Text("\(account.provider.rawValue)/")
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.secondary)
                TextField("folder name", text: Binding(
                    get: { editingSlug ?? "" },
                    set: { editingSlug = $0 }
                ))
                .textFieldStyle(.roundedBorder)
                .font(.system(size: 11, design: .monospaced))
                .onSubmit(commitRename)
                Button("Save", action: commitRename)
                    .controlSize(.small)
                    .disabled(state.isBusy)
                Button("Cancel") { editingSlug = nil }.controlSize(.small).buttonStyle(.plain).foregroundStyle(.secondary)
            }
            if let renameError {
                Text(renameError).font(.caption2).foregroundStyle(.red)
            }
        }
        .font(.caption)
    }

    private func commitRename() {
        guard let slug = editingSlug, !state.isBusy else { return }
        renameError = nil
        Task {
            do {
                try await store.rename(account.id, to: slug)
                editingSlug = nil
            } catch {
                renameError = error.localizedDescription
            }
        }
    }

    private func secondaryWindows(_ quota: QuotaSnapshot) -> [QuotaWindow] {
        quota.windows.filter { $0 != quota.weekly }
    }

    private func resetText(for window: QuotaWindow) -> String {
        guard let date = window.resetsAt else { return "Reset date unavailable" }
        return "Resets \(date.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day().hour().minute()))"
    }
}

private struct QuotaBar: View {
    let remaining: Int
    let color: Color

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .leading) {
                Capsule().fill(.secondary.opacity(0.16))
                Capsule()
                    .fill(color)
                    .frame(width: geometry.size.width * CGFloat(remaining) / 100)
            }
        }
        .frame(height: 4)
        .animation(.easeOut(duration: 0.35), value: remaining)
    }
}

private func color(forRemaining percent: Int) -> Color {
    switch percent {
    case 50...: return Color(red: 0.32, green: 0.62, blue: 0.46)
    case 25..<50: return Color(red: 0.76, green: 0.58, blue: 0.31)
    default: return Color(red: 0.72, green: 0.34, blue: 0.35)
    }
}

@MainActor
private func copyToPasteboard(_ text: String) {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(text, forType: .string)
}

private struct TahoeTranslucentBackground: NSViewRepresentable {
    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = .popover
        view.blendingMode = .behindWindow
        view.state = .active
        return view
    }

    func updateNSView(_ nsView: NSVisualEffectView, context: Context) {}
}

private struct LoginBanner: View {
    let instructions: LoginInstructions
    let onSubmitCode: (String) async -> Bool
    let onCancel: () -> Void

    @State private var code = ""
    @State private var submitted = false
    @State private var submitting = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                ProgressView().controlSize(.small)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Finish connecting in your browser")
                        .font(.caption.weight(.medium))
                    Button("Open sign-in again") { NSWorkspace.shared.open(instructions.authURL) }
                        .buttonStyle(.link)
                        .font(.caption2)
                }
                Spacer()
                Button("Cancel", action: onCancel)
                    .buttonStyle(.plain)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if instructions.acceptsCode {
                HStack(spacing: 6) {
                    TextField("Paste the code shown after sign-in", text: $code)
                        .textFieldStyle(.roundedBorder)
                        .font(.caption)
                        .onSubmit(submit)
                    Button(submitted ? "Sent" : "Submit", action: submit)
                        .controlSize(.small)
                        .disabled(code.trimmingCharacters(in: .whitespaces).isEmpty || submitting || submitted)
                }
            }
        }
        .padding(13)
    }

    private func submit() {
        let trimmed = code.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !submitting, !submitted else { return }
        submitting = true
        Task {
            submitted = await onSubmitCode(trimmed)
            if submitted { code = "" }
            submitting = false
        }
    }
}
