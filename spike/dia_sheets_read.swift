wake()
let w0 = (attr(ax,"AXWindows") as! [AXUIElement])[0]
var tabs: [AXUIElement] = []
findAll(w0, { s($0,"AXSubrole") == "AXTabButton" && s($0,"AXTitle") == "spike-11-synthetic" }, &tabs, 6)
if let t = tabs.first { _ = AXUIElementPerformAction(t, "AXPress" as CFString) }
Thread.sleep(forTimeInterval: 0.8)
var tw = now()
let wa = webArea()!
print(String(format: "locate web area %.1f ms", now()-tw))
tw = now()
let nameBox = find(wa, { s($0,"AXDOMIdentifier") == "t-name-box" })!
let fbar = find(wa, { s($0,"AXDOMIdentifier") == "t-formula-bar-input" })!
print(String(format: "locate name box + formula bar %.1f ms", now()-tw))
var fbTA: AXUIElement? = nil
func fbText() -> String {
  if fbTA == nil { var ta: [AXUIElement] = []; findAll(fbar, { s($0,"AXRole") == "AXTextArea" }, &ta, 4); fbTA = ta.first }
  return fbTA.flatMap { attr($0,"AXValue") as? String } ?? ""
}
func nbText() -> String { (attr(nameBox,"AXValue") as? String) ?? "" }
let src = CGEventSource(stateID: .hidSystemState)
func key(_ k: CGKeyCode) { for d in [true,false] { let e = CGEvent(keyboardEventSource: src, virtualKey: k, keyDown: d)!; e.postToPid(app.processIdentifier) } }
print("selection now: namebox=\(nbText()) fbar=\(fbText())")
tw = now(); let _ = nbText(); let _ = fbText(); print(String(format: "read selected cell (name box + formula bar, handles cached): %.2f ms", now()-tw))
let csv: [[String]] = try! String(contentsOfFile: "/Users/shin/Desktop/projects/desktop-ai-companion/.claude/worktrees/agent-a5ee9034f19414394/stand-in/data/staff.csv", encoding: .utf8).split(separator: "\n").map { $0.split(separator: ",", omittingEmptySubsequences: false).map(String.init) }
func clean(_ x: String) -> String { x.trimmingCharacters(in: .whitespacesAndNewlines) }
let cols = ["A","B","C","D"]
var grid: [[String]] = []; var settle: [Double] = []; var ok = 0; var stale = 0
let t5 = now()
var prev = clean(fbText())
for r in 2...6 { var row: [String] = []
  for (ci, c) in cols.enumerated() {
    let t = now()
    _ = AXUIElementSetAttributeValue(nameBox, "AXFocused" as CFString, kCFBooleanTrue)
    _ = AXUIElementSetAttributeValue(nameBox, "AXValue" as CFString, "\(c)\(r)" as CFString)
    key(36)
    var v = prev
    let dl = now() + 3000
    while now() < dl { v = clean(fbText()); if nbText() == "\(c)\(r)" && v != prev { break }; Thread.sleep(forTimeInterval: 0.002) }
    settle.append(now()-t)
    if v == csv[r-1][ci] { ok += 1 }
    if v == prev { stale += 1 }
    prev = v; row.append(v) }
  grid.append(row) }
let total = now() - t5
print(String(format: "5x4 read (A2:D6) by navigating: %.1f ms total, %.1f ms/cell, min %.1f max %.1f; matches csv %d/20; unchanged-after-timeout %d", total, total/20, settle.min()!, settle.max()!, ok, stale))
for r in grid { print(r) }
