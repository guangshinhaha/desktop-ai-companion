import Cocoa
import ApplicationServices
func now() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1e6 }
func attr(_ e: AXUIElement, _ n: String) -> AnyObject? { var v: CFTypeRef?; return AXUIElementCopyAttributeValue(e, n as CFString, &v) == .success ? (v as AnyObject?) : nil }
func s(_ e: AXUIElement, _ n: String) -> String { (attr(e, n) as? String) ?? "" }
let app = NSRunningApplication.runningApplications(withBundleIdentifier: "company.thebrowser.dia").first!
let ax = AXUIElementCreateApplication(app.processIdentifier)
let args = CommandLine.arguments
let maxD = args.count > 1 ? Int(args[1])! : 6
if args.contains("--wake") {
  print("AXEnhancedUserInterface", AXUIElementSetAttributeValue(ax, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue).rawValue)
  print("AXManualAccessibility", AXUIElementSetAttributeValue(ax, "AXManualAccessibility" as CFString, kCFBooleanTrue).rawValue)
  Thread.sleep(forTimeInterval: 1.5)
}
var count = 0
var roles: [String: Int] = [:]
func dump(_ e: AXUIElement, _ d: Int, _ inWeb: Bool) {
  let r = s(e, "AXRole"); if r == "AXMenuBar" { return }
  roles[r, default: 0] += 1; count += 1
  var web = inWeb
  if r == "AXWebArea" { web = true }
  if d <= maxD && (web || d < 4) {
    let t = s(e, "AXTitle"), de = s(e, "AXDescription"), id = s(e, "AXDOMIdentifier")
    print(String(repeating: " ", count: d) + "\(r) sub=\(s(e,"AXSubrole")) t=\(String(t.prefix(40))) d=\(String(de.prefix(60))) id=\(id)")
  }
  for c in (attr(e, "AXChildren") as? [AXUIElement]) ?? [] { dump(c, d+1, web) }
}
let t = now()
for w in (attr(ax,"AXWindows") as? [AXUIElement]) ?? [] { dump(w, 0, false) }
print(String(format: "walk %.1f ms, %d elements", now()-t, count))
print(roles.sorted { $0.value > $1.value }.prefix(12))
