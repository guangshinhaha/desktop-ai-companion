import Foundation

/// What the officer can do to a run in progress (PRD C-3, C-4): pause it or stop it. The shell sends
/// `replay.pause`, `replay.resume` and `replay.stop` as notifications on the executor's stdin.
///
/// The replayer looks at this at every checkpoint (between steps, before it acts, and in every wait,
/// which is cut into short slices), so a stop is seen within one slice and nothing is done after it.
/// A stop is final. A pause holds the run before its next step; it does not interrupt a step that is
/// already waiting for its checks. When the shell's end closes while paused, the run stops.
public final class RunControl {
    public enum State: Equatable { case running, paused, stopped }

    private let lock = NSLock()
    private var current: State = .running
    private let pollInput: (() -> String?)?
    private let blockingInput: (() -> String?)?

    /// `poll` returns a pending line without waiting, or nil. `block` waits for the next line (used only
    /// while paused, so an idle paused run costs nothing) and returns nil when the shell's end closes.
    public init(poll: (() -> String?)? = nil, block: (() -> String?)? = nil) {
        self.pollInput = poll; self.blockingInput = block
    }

    public var state: State { lock.lock(); defer { lock.unlock() }; return current }

    public func pause() { lock.lock(); if current == .running { current = .paused }; lock.unlock() }
    public func resume() { lock.lock(); if current == .paused { current = .running }; lock.unlock() }
    public func stop() { lock.lock(); current = .stopped; lock.unlock() }

    /// Applies one input line if it is a control message. Returns whether it was one.
    @discardableResult
    public func apply(line: String) -> Bool {
        guard let o = RPC.parse(line), let m = o["method"] as? String else { return false }
        switch m {
        case "replay.pause": pause()
        case "replay.resume": resume()
        case "replay.stop": stop()
        default: return false
        }
        return true
    }

    /// Reads whatever the shell has already sent, without waiting.
    public func checkpoint() {
        guard let pollInput else { return }
        for _ in 0..<64 { guard let line = pollInput() else { return }; apply(line: line) }
    }

    /// True once a stop has been asked for. Looks at the shell's pending input first.
    public var isStopped: Bool { checkpoint(); return state == .stopped }

    /// Holds here while paused. Returns when resumed or stopped.
    public func holdWhilePaused(onPause: () -> Void, onResume: () -> Void) {
        checkpoint()
        guard state == .paused else { return }
        onPause()
        while state == .paused {
            guard let line = blockingInput?() else { stop(); break }
            apply(line: line)
        }
        if state == .running { onResume() }
    }
}

/// Reads lines from a file descriptor either by waiting or only if some are already there.
/// Used on standard input, so a stop sent while a step waits is seen without a second thread.
public final class LineReader {
    private let fd: Int32
    private var buffer = [UInt8]()
    private var ended = false

    public init(fd: Int32) { self.fd = fd }

    private func takeLine() -> String? {
        guard let i = buffer.firstIndex(of: 10) else { return nil }
        let line = String(decoding: buffer[..<i], as: UTF8.self)
        buffer.removeSubrange(...i)
        return line
    }

    private func fill() {
        var chunk = [UInt8](repeating: 0, count: 4096)
        while true {
            let n = read(fd, &chunk, chunk.count)
            if n > 0 { buffer.append(contentsOf: chunk[..<n]); return }
            if n < 0 && errno == EINTR { continue }
            ended = true
            return
        }
    }

    /// The next line, waiting for it. nil at end of input.
    public func readLine() -> String? {
        while true {
            if let l = takeLine() { return l }
            if ended {
                if buffer.isEmpty { return nil }
                defer { buffer.removeAll() }
                return String(decoding: buffer, as: UTF8.self)
            }
            fill()
        }
    }

    /// The next line if one is already waiting, else nil at once.
    public func pollLine() -> String? {
        if let l = takeLine() { return l }
        guard !ended else { return nil }
        var p = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
        guard Foundation.poll(&p, 1, 0) > 0 else { return nil }
        fill()
        return takeLine()
    }
}
