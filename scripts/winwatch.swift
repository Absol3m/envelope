import Foundation
import CoreGraphics
// winwatch <pid> : attend la première fenêtre (>= 400 px de large) du processus et affiche son identifiant et le délai.
let pid = Int32(CommandLine.arguments[1])!
let start = Date()
while Date().timeIntervalSince(start) < 60 {
    if let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] {
        for w in list where (w[kCGWindowOwnerPID as String] as? Int32) == pid {
            let b = w[kCGWindowBounds as String] as? [String: Any]
            if let width = b?["Width"] as? Double, width >= 400, (w[kCGWindowLayer as String] as? Int) == 0 {
                print("\(Int(Date().timeIntervalSince(start) * 1000)) \(w[kCGWindowNumber as String] ?? 0)")
                exit(0)
            }
        }
    }
    usleep(10_000)
}
print("-1 0")
