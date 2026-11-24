import Cocoa
import ApplicationServices
func now() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1e6 }
func attr(_ e: AXUIElement, _ n: String) -> AnyObject? { var v: CFTypeRef?; return AXUIElementCopyAttributeValue(e, n as CFString, &v) == .success ? (v as AnyObject?) : nil }
func s(_ e: AXUIElement, _ n: String) -> String { (attr(e, n) as? String) ?? "" }
func kids(_ e: AXUIElement) -> [AXUIElement] { (attr(e, "AXChildren") as? [AXUIElement]) ?? [] }
let app = NSRunningApplication.runningApplications(withBundleIdentifier: "company.thebrowser.dia").first!
let ax = AXUIElementCreateApplication(app.processIdentifier)
func wake() {
  _ = AXUIElementSetAttributeValue(ax, "AXManualAccessibility" as CFString, kCFBooleanTrue)
  _ = AXUIElementSetAttributeValue(ax, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
}
func webArea() -> AXUIElement? {
  for w in (attr(ax,"AXWindows") as? [AXUIElement]) ?? [] {
    var stack = [w]; var depth = 0
    // BFS limited to first levels
    var level = [w]
    while depth < 5 && !level.isEmpty {
      var next: [AXUIElement] = []
      for e in level { if s(e,"AXRole") == "AXWebArea" && s(e,"AXTitle").contains("Google Sheets") { return e }; next += kids(e) }
      level = next; depth += 1
    }
    _ = stack
  }
  return nil
}
func find(_ e: AXUIElement, _ pred: (AXUIElement) -> Bool, _ maxD: Int = 30) -> AXUIElement? {
  if pred(e) { return e }
  if maxD == 0 { return nil }
  for c in kids(e) { if let r = find(c, pred, maxD-1) { return r } }
  return nil
}
func findAll(_ e: AXUIElement, _ pred: (AXUIElement) -> Bool, _ out: inout [AXUIElement], _ maxD: Int = 40) {
  if pred(e) { out.append(e) }
  if maxD == 0 { return }
  for c in kids(e) { findAll(c, pred, &out, maxD-1) }
}
