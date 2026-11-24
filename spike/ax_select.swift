// Throwaway spike: after enabling the tree, read selected cell address+value and time reads. Not product code.
import Cocoa
import ApplicationServices
func now() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1e6 }
func attr(_ e: AXUIElement, _ n: String) -> AnyObject? { var v: CFTypeRef?; return AXUIElementCopyAttributeValue(e, n as CFString, &v) == .success ? (v as AnyObject?) : nil }
func s(_ e: AXUIElement, _ n: String) -> String { (attr(e, n) as? String) ?? "" }
func find(_ e: AXUIElement, _ role: String) -> AXUIElement? {
  if s(e,"AXRole") == "AXMenuBar" { return nil }
  if s(e,"AXRole") == role { return e }
  for c in (attr(e,"AXChildren") as? [AXUIElement]) ?? [] { if let f = find(c, role) { return f } }
  return nil
}
let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.iWork.Numbers").first!
let ax = AXUIElementCreateApplication(app.processIdentifier)
_ = AXUIElementSetAttributeValue(ax, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
Thread.sleep(forTimeInterval: 1.5)
var t = now()
guard let table = find(ax, "AXTable") else { print("no AXTable"); exit(1) }
print(String(format: "locate table: %.1f ms  title=%@", now()-t, s(table,"AXTitle")))
var names: CFArray?; AXUIElementCopyAttributeNames(table, &names); print("table attrs: \(names as? [String] ?? [])")
t = now()
let sel = attr(table, "AXSelectedCells") as? [AXUIElement] ?? []
print(String(format: "AXSelectedCells: %.2f ms, count=%d", now()-t, sel.count))
for c in sel {
  var cn: CFArray?; AXUIElementCopyAttributeNames(c, &cn)
  print("selected cell attrs: \(cn as? [String] ?? [])")
  print("  title=\(s(c,"AXTitle")) desc=\(s(c,"AXDescription")) value=\(attr(c,"AXValue").map{"\($0)"} ?? "nil") rowidx/colidx=\(attr(c,"AXRowIndexRange").map{"\($0)"} ?? "nil")/\(attr(c,"AXColumnIndexRange").map{"\($0)"} ?? "nil") id=\(s(c,"AXIdentifier"))")
}
func cellInfo(_ c: AXUIElement) -> String {
  "title=\(s(c,"AXTitle")) desc=\(s(c,"AXDescription")) value=\(attr(c,"AXValue").map{"\($0)"} ?? "nil") rowRange=\(attr(c,"AXRowIndexRange").map{"\($0)"} ?? "nil") colRange=\(attr(c,"AXColumnIndexRange").map{"\($0)"} ?? "nil")"
}
// Full-table read via AXChildren (rows -> cells)
t = now()
var rows: [[String]] = []
var cellEls: [[AXUIElement]] = []
for row in (attr(table,"AXChildren") as? [AXUIElement]) ?? [] {
  let cells = (attr(row,"AXChildren") as? [AXUIElement]) ?? []
  cellEls.append(cells)
  let vals = cells.map { s($0,"AXTitle") }
  if vals.contains(where: {!$0.isEmpty}) { rows.append(vals) }
}
print(String(format: "read all rows via AX: %.1f ms, non-empty rows=%d", now()-t, rows.count))
rows.prefix(3).forEach { print($0) }
var cn: CFArray?; AXUIElementCopyAttributeNames(cellEls[2][1], &cn); print("cell attrs: \(cn as? [String] ?? [])")
print("cell r3c2:", cellInfo(cellEls[2][1]))
// Select cell (row3,col2) via AX, then read selection back
t = now()
let setR = AXUIElementSetAttributeValue(table, "AXSelectedCells" as CFString, [cellEls[2][1]] as CFArray)
print(String(format: "set AXSelectedCells err=%d %.2f ms", setR.rawValue, now()-t))
Thread.sleep(forTimeInterval: 0.3)
t = now()
let sel2 = attr(table, "AXSelectedCells") as? [AXUIElement] ?? []
print(String(format: "read AXSelectedCells: %.2f ms count=%d", now()-t, sel2.count))
sel2.forEach { print("selected:", cellInfo($0)) }
// parameterized
var out: CFTypeRef?
let r = AXUIElementCopyParameterizedAttributeValue(table, "AXCellForColumnAndRow" as CFString, [1, 2] as CFArray, &out)
print("AXCellForColumnAndRow err=\(r.rawValue)")
