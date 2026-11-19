import Darwin
import Foundation

/// Coordinates a single child and both of its output pipes on a private queue.
final class CLIProcess: @unchecked Sendable {
    struct Result: Sendable {
        let stdout: Data
        let stderr: Data
        let status: Int32
    }

    private enum Output: CaseIterable {
        case stdout, stderr
    }

    private let queue = DispatchQueue(label: "Codesubs.CLIProcess")
    private let process = Process()
    private let pipes: [Output: Pipe] = [.stdout: Pipe(), .stderr: Pipe()]
    private let input: Data?
    private let inputPipe: Pipe?
    private let timeout: TimeInterval
    private let onLine: (@Sendable (Data) throws -> Void)?
    private var readers: [Output: DispatchSourceRead] = [:]
    private var stdout = Data()
    private var stderr = Data()
    private var pendingLine = Data()
    private var continuation: CheckedContinuation<Result, Error>?
    private var status: Int32?
    private var pendingError: Error?
    private var finished = false

    init(
        executable: URL,
        arguments: [String],
        input: Data? = nil,
        timeout: TimeInterval,
        onLine: (@Sendable (Data) throws -> Void)?
    ) {
        process.executableURL = executable
        process.arguments = arguments
        self.input = input
        let inputPipe = input.map { _ in Pipe() }
        self.inputPipe = inputPipe
        if let inputPipe {
            process.standardInput = inputPipe
        } else {
            process.standardInput = FileHandle.nullDevice
        }
        process.standardOutput = pipes[.stdout]
        process.standardError = pipes[.stderr]
        self.timeout = timeout
        self.onLine = onLine
    }

    func run() async throws -> Result {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                queue.async {
                    self.start(continuation)
                }
            }
        } onCancel: {
            queue.async {
                self.abort(CancellationError())
            }
        }
    }

    private func start(_ continuation: CheckedContinuation<Result, Error>) {
        if let pendingError {
            continuation.resume(throwing: pendingError)
            return
        }
        self.continuation = continuation
        process.terminationHandler = { [weak self] child in
            let status = child.terminationStatus
            self?.queue.async { [weak self] in
                self?.status = status
                self?.finishIfDone()
            }
        }

        do {
            if let writer = inputPipe?.fileHandleForWriting,
               fcntl(writer.fileDescriptor, F_SETNOSIGPIPE, 1) == -1 {
                throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
            }
            for output in Output.allCases {
                guard let handle = pipes[output]?.fileHandleForReading else { continue }
                let descriptor = handle.fileDescriptor
                guard fcntl(descriptor, F_SETFL, O_NONBLOCK) != -1 else {
                    throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
                }
                let reader = DispatchSource.makeReadSource(fileDescriptor: descriptor, queue: queue)
                reader.setEventHandler { [weak self] in self?.consume(output, from: descriptor) }
                reader.setCancelHandler { try? handle.close() }
                readers[output] = reader
                reader.resume()
            }
            try process.run()
            if let input, let writer = inputPipe?.fileHandleForWriting {
                DispatchQueue.global(qos: .utility).async {
                    defer { try? writer.close() }
                    try? writer.write(contentsOf: input)
                }
            }
            queue.asyncAfter(deadline: .now() + timeout) { [weak self] in
                self?.abort(SubscriptionClientError.timedOut)
            }
        } catch {
            finish(.failure(SubscriptionClientError.launchFailed(error.localizedDescription)))
        }
    }

    private func consume(_ output: Output, from descriptor: Int32) {
        guard !finished, readers[output] != nil else { return }

        var bytes = [UInt8](repeating: 0, count: 16_384)
        let count = read(descriptor, &bytes, bytes.count)
        if count < 0 {
            let error = errno
            if error != EAGAIN, error != EINTR {
                readers.removeValue(forKey: output)?.cancel()
                abort(POSIXError(POSIXErrorCode(rawValue: error) ?? .EIO))
            }
            return
        }

        do {
            if count == 0 {
                if pendingError == nil, output == .stdout, !pendingLine.isEmpty {
                    try onLine?(pendingLine)
                    pendingLine.removeAll()
                }
                readers.removeValue(forKey: output)?.cancel()
                finishIfDone()
            } else if pendingError != nil {
                return
            } else if output == .stderr {
                stderr.append(contentsOf: bytes.prefix(count))
            } else if let onLine {
                try consumeLines(bytes.prefix(count), onLine: onLine)
            } else {
                stdout.append(contentsOf: bytes.prefix(count))
            }
        } catch {
            abort(error)
        }
    }

    private func consumeLines(
        _ bytes: ArraySlice<UInt8>,
        onLine: @Sendable (Data) throws -> Void
    ) throws {
        var start = bytes.startIndex
        while let newline = bytes[start...].firstIndex(of: 10) {
            pendingLine.append(contentsOf: bytes[start..<newline])
            try onLine(pendingLine)
            pendingLine.removeAll(keepingCapacity: true)
            start = bytes.index(after: newline)
        }
        pendingLine.append(contentsOf: bytes[start...])
    }

    private func finishIfDone() {
        if let pendingError, status != nil {
            finish(.failure(pendingError))
        } else if let status, readers.isEmpty {
            finish(.success(Result(stdout: stdout, stderr: stderr, status: status)))
        }
    }

    private func abort(_ error: Error) {
        guard !finished, pendingError == nil else { return }
        pendingError = error
        // Cancellation can reach the queue before the continuation does.
        guard continuation != nil else { return }
        guard process.isRunning else {
            finish(.failure(error))
            return
        }
        process.terminate()
        // Give the CLI time to roll back interrupted login and account changes.
        queue.asyncAfter(deadline: .now() + .seconds(8)) { [weak self] in
            guard let self, !self.finished else { return }
            if self.process.isRunning { kill(self.process.processIdentifier, SIGKILL) }
            self.finish(.failure(error))
        }
    }

    private func finish(_ result: Swift.Result<Result, Error>) {
        guard !finished else { return }
        finished = true
        readers.values.forEach { $0.cancel() }
        readers.removeAll()
        process.terminationHandler = nil
        let waiting = continuation
        continuation = nil
        waiting?.resume(with: result)
    }
}
