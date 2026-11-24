// Throwaway spike: dump Numbers window AX subtree (skip menus), optionally set AXEnhancedUserInterface.
import Cocoa
import ApplicationServices
func now() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1e6 }
func attr(_ e: AXUIElement, _ n: String) -> AnyObject? { var v: CFTypeRef?; return AXUIElementCopyAttributeValue(e, n as CFString, &v) == .success ? (v as AnyObject?) : nil }
func s(_ e: AXUIElement, _ n: String) -> String { (attr(e, n) as? String) ?? "" }
let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.iWork.Numbers").first!
let ax = AXUIElementCreateApplication(app.processIdentifier)
if CommandLine.arguments.contains("--enhance") {
  print("set AXEnhancedUserInterface:", AXUIElementSetAttributeValue(ax, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue).rawValue)
  print("set AXManualAccessibility:", AXUIElementSetAttributeValue(ax, "AXManualAccessibility" as CFString, kCFBooleanTrue).rawValue)
  Thread.sleep(forTimeInterval: 1.5)
}
func dump(_ e: AXUIElement, _ d: Int) {
  let r = s(e, "AXRole"); if r == "AXMenuBar" { return }
  var v = ""; if let x = attr(e, "AXValue") { v = "\(x)" }
  print(String(repeating: "  ", count: d) + "\(r) sub=\(s(e,"AXSubrole")) t=\(s(e,"AXTitle")) d=\(s(e,"AXDescription")) v=\(v)")
  for c in (attr(e, "AXChildren") as? [AXUIElement]) ?? [] { dump(c, d+1) }
}
let t = now(); dump(ax, 0); print(String(format: "dump %.1f ms", now()-t))
