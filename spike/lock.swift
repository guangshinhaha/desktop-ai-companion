import Foundation
import CoreGraphics
let d = CGSessionCopyCurrentDictionary() as? [String:Any] ?? [:]
print("CGSSessionScreenIsLocked=\(d["CGSSessionScreenIsLocked"] ?? 0)")
