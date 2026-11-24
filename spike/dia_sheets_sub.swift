wake()
guard let wa = webArea() else { print("no web area"); exit(1) }
let id = CommandLine.arguments[1]
let md = Int(CommandLine.arguments[2])!
guard let root = find(wa, { s($0,"AXDOMIdentifier") == id }) else { print("id not found"); exit(1) }
func d(_ e: AXUIElement, _ k: Int) {
  var v = ""; if let x = attr(e, "AXValue") { v = "\(x)" }
  print(String(repeating: " ", count: k) + "\(s(e,"AXRole")) sub=\(s(e,"AXSubrole")) t=\(String(s(e,"AXTitle").prefix(50))) d=\(String(s(e,"AXDescription").prefix(50))) v=\(String(v.prefix(50))) id=\(s(e,"AXDOMIdentifier"))")
  if k < md { for c in kids(e) { d(c, k+1) } }
}
d(root, 0)
