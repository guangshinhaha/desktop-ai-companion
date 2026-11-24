/// A read-only snapshot of one element in an app's accessibility tree.
/// The resolver works on snapshots, so it can be tested without any app running.
/// Field values are never copied into a snapshot (PDPA: store roles and labels, not values).
public final class UINode {
    public let role: String
    public let label: String?
    public let domId: String?
    public internal(set) var children: [UINode]
    public internal(set) weak var parent: UINode?
    /// The live element behind the snapshot (an `AXUIElement` in the macOS adapter).
    public let handle: Any?
    /// The accessibility subrole (`AXDialog`, `AXApplicationDialog`), where the app gives one.
    public let subrole: String?

    public init(role: String, label: String? = nil, domId: String? = nil,
                children: [UINode] = [], handle: Any? = nil, subrole: String? = nil) {
        self.subrole = subrole
        self.role = role; self.label = label; self.domId = domId
        self.children = children; self.handle = handle
        for c in children { c.parent = self }
    }

    /// All nodes below this one, document order, not including this node.
    public var descendants: [UINode] {
        var out: [UINode] = []
        var stack = Array(children.reversed())
        while let n = stack.popLast() {
            out.append(n)
            stack.append(contentsOf: n.children.reversed())
        }
        return out
    }

    func ancestors() -> [UINode] {
        var out: [UINode] = []
        var p = parent
        while let n = p { out.append(n); p = n.parent }
        return out
    }
}
