// Throwaway spike: read cells from Numbers through AXUIElement. Not product code.
import Cocoa
import ApplicationServices
func now() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1e6 }
func attr(_ e: AXUIElement, _ n: String) -> AnyObject? {
  var v: CFTypeRef?; return AXUIElementCopyAttributeValue(e, n as CFString, &v) == .success ? (v as AnyObject?) : nil
}
func kids(_ e: AXUIElement) -> [AXUIElement] { (attr(e, kAXChildrenAttribute) as? [AXUIElement]) ?? [] }
func str(_ e: AXUIElement, _ n: String) -> String { (attr(e, n) as? String) ?? "" }
print("trusted=\(AXIsProcessTrusted())")
guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.iWork.Numbers").first else { print("Numbers not running"); exit(1) }
let ax = AXUIElementCreateApplication(app.processIdentifier)
var counts: [String:Int] = [:]
var sample: [String] = []
var maxDepth = 0
func walk(_ e: AXUIElement, _ d: Int) {
  maxDepth = max(maxDepth, d)
  let r = str(e, kAXRoleAttribute); counts[r, default: 0] += 1
  if sample.count < 40 && ["AXCell","AXTable","AXRow","AXColumn","AXScrollArea","AXGroup"].contains(r) {
    sample.append("d\(d) \(r) title=\(str(e,"AXTitle")) desc=\(str(e,"AXDescription")) val=\(attr(e,kAXValueAttribute).map{"\($0)"} ?? "nil") id=\(str(e,"AXIdentifier"))")
  }
  for c in kids(e) { walk(c, d+1) }
}
let t0 = now()
walk(ax, 0)
print(String(format: "full tree walk: %.1f ms, maxDepth=%d", now()-t0, maxDepth))
print("roles: \(counts.sorted{$0.value>$1.value}.prefix(15))")
sample.forEach { print($0) }
// Focused element + selected cells
let t1 = now()
let focused = attr(ax, kAXFocusedUIElementAttribute)
print(String(format: "focused element lookup: %.2f ms", now()-t1))
if let f = focused { let fe = f as! AXUIElement
  print("focused role=\(str(fe,"AXRole")) desc=\(str(fe,"AXDescription")) val=\(attr(fe,kAXValueAttribute).map{"\($0)"} ?? "nil")")
  var names: CFArray?; AXUIElementCopyAttributeNames(fe, &names); print("focused attrs: \(names as? [String] ?? [])")
}
