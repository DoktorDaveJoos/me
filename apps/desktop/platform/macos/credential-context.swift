// Explicit window previews, local interpretation and reviewed form filling.
// No network, clipboard, disk captures, page scripts or form submission.
import AppKit
import ApplicationServices
import CryptoKit
import Foundation
import ScreenCaptureKit
import Vision

func emit(_ value: Any) {
    if let data = try? JSONSerialization.data(withJSONObject: value) { FileHandle.standardOutput.write(data) }
}
func fail(_ code: String) -> Never { emit(["error": code]); exit(0) }
func read(_ element: AXUIElement, _ attribute: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success ? value : nil
}
func string(_ element: AXUIElement, _ attribute: String) -> String {
    let value = read(element, attribute)
    return (value as? String) ?? (value as? URL)?.absoluteString ?? ""
}
func elements(_ element: AXUIElement, _ attribute: String) -> [AXUIElement] {
    read(element, attribute) as? [AXUIElement] ?? []
}
func label(_ element: AXUIElement) -> String {
    for attribute in [kAXTitleAttribute, kAXDescriptionAttribute, "AXPlaceholderValue"] {
        let value = string(element, attribute).trimmingCharacters(in: .whitespacesAndNewlines)
        if !value.isEmpty { return String(value.prefix(512)) }
    }
    if let title = read(element, "AXTitleUIElement"), CFGetTypeID(title) == AXUIElementGetTypeID() {
        return String(string(unsafeBitCast(title, to: AXUIElement.self), kAXValueAttribute).prefix(512))
    }
    return ""
}
// Capture/fill are quiet checks. Only the explicit Settings action requests access.
func trusted() -> Bool { AXIsProcessTrusted() }
func accessibility(requestPermission: Bool) {
    let granted = requestPermission
        ? AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary)
        : trusted()
    // The permission prompt is asynchronous; its return value is not a grant.
    emit(["accessibility": granted])
}
func input() -> [String: Any] {
    let data = FileHandle.standardInput.readData(ofLength: 65537)
    guard data.count <= 65536, let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { fail("invalid") }
    return value
}
func windowBounds(_ element: AXUIElement) -> CGRect? {
    guard let p = read(element, kAXPositionAttribute), let s = read(element, kAXSizeAttribute),
          CFGetTypeID(p) == AXValueGetTypeID(), CFGetTypeID(s) == AXValueGetTypeID() else { return nil }
    var point = CGPoint.zero; var size = CGSize.zero
    guard AXValueGetValue(unsafeBitCast(p, to: AXValue.self), .cgPoint, &point),
          AXValueGetValue(unsafeBitCast(s, to: AXValue.self), .cgSize, &size) else { return nil }
    return CGRect(origin: point, size: size)
}
func sameBounds(_ a: CGRect, _ b: CGRect) -> Bool {
    abs(a.minX-b.minX) < 2 && abs(a.minY-b.minY) < 2 && abs(a.width-b.width) < 2 && abs(a.height-b.height) < 2
}
func selectedWindow(_ source: [String: Any]) -> (NSRunningApplication, AXUIElement) {
    guard trusted() else { fail("permission") }
    guard let pid = source["pid"] as? Int32, let bundle = source["bundle"] as? String,
          let windowID = source["id"] as? UInt32,
          let running = NSRunningApplication(processIdentifier: pid), running.bundleIdentifier == bundle,
          let rows = CGWindowListCopyWindowInfo([.optionIncludingWindow], windowID) as? [[String: Any]],
          let row = rows.first, row[kCGWindowOwnerPID as String] as? Int32 == pid,
          let boundsData = row[kCGWindowBounds as String] as? [String: Any],
          let bounds = CGRect(dictionaryRepresentation: boundsData as CFDictionary) else { fail("window_identity") }
    let app = AXUIElementCreateApplication(pid)
    AXUIElementSetMessagingTimeout(app, 0.2)
    let title = row[kCGWindowName as String] as? String ?? ""
    let candidates = elements(app, kAXWindowsAttribute).filter {
        windowBounds($0).map { sameBounds($0, bounds) } ?? false
    }
    if candidates.count == 1 { return (running, candidates[0]) }
    let named = candidates.filter { !title.isEmpty && (string($0, kAXTitleAttribute) == title || string($0, kAXTitleAttribute).hasPrefix(title + " - ")) }
    guard named.count == 1 else { fail("window_match") }
    return (running, named[0])
}
struct Node {
    let element: AXUIElement
    let path: [Int]
    let role: String
    let subrole: String
    let label: String
}
func walk(_ root: AXUIElement, stopAtWebArea: Bool = false) -> ([Node], Bool) {
    var queue: [(AXUIElement, [Int])] = [(root, [])]
    var result: [Node] = []; var cursor = 0
    let deadline = Date().addingTimeInterval(4)
    while cursor < queue.count && cursor < 1000 && Date() < deadline {
        let (element, path) = queue[cursor]; cursor += 1
        AXUIElementSetMessagingTimeout(element, 0.1)
        let role = string(element, kAXRoleAttribute)
        result.append(Node(element: element, path: path, role: role, subrole: string(element, kAXSubroleAttribute), label: label(element)))
        if path.count < 20 && !(stopAtWebArea && !path.isEmpty && role == "AXWebArea") {
            for (index, child) in elements(element, kAXChildrenAttribute).prefix(max(0, 1000-queue.count)).enumerated() {
                queue.append((child, path + [index]))
            }
        }
    }
    return (result, cursor < queue.count)
}
func documentURL(_ node: AXUIElement) -> String {
    let document = string(node, kAXDocumentAttribute)
    return document.isEmpty ? string(node, "AXURL") : document
}
func origin(_ value: String) -> String? {
    guard let u = URLComponents(string: value), ["https", "http"].contains(u.scheme ?? ""),
          let host = u.host, !host.isEmpty, u.user == nil, u.password == nil else { return nil }
    let port = u.port.map { ":\($0)" } ?? ""
    return "\(u.scheme!)://\(host)\(port)"
}
func semantic(_ node: Node) -> String? {
    let name = node.label.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: " *:"))
    if ["email", "email address", "e-mail", "e-mail address", "username", "user name"].contains(name) { return "username" }
    if ["name", "full name", "your name"].contains(name) { return "full_name" }
    if node.subrole == kAXSecureTextFieldSubrole || ["password", "new password", "confirm password", "confirm new password", "password confirmation", "repeat password"].contains(name) { return "password" }
    return nil
}
func fields(_ nodes: [Node]) -> [Node] { nodes.filter { $0.role == kAXTextFieldRole || $0.role == kAXTextAreaRole } }
func settable(_ element: AXUIElement) -> Bool {
    var value: DarwinBoolean = false
    return AXUIElementIsAttributeSettable(element, kAXValueAttribute as CFString, &value) == .success && value.boolValue
}
func fingerprint(_ fields: [Node]) -> String {
    let values = fields.map { ["path": $0.path, "role": $0.role, "subrole": $0.subrole, "label": $0.label] as [String: Any] }
    let data = (try? JSONSerialization.data(withJSONObject: values, options: .sortedKeys)) ?? Data()
    return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}
func form(_ window: AXUIElement) -> (String, [Node], Bool) {
    let (all, truncated) = walk(window)
    let areas = all.filter { $0.role == "AXWebArea" && origin(documentURL($0.element)) != nil }
    guard let area = areas.first else { return ("", all, truncated) }
    let (nodes, partial) = walk(area.element, stopAtWebArea: true)
    return (documentURL(area.element), nodes.filter { $0.role != "AXWebArea" }, truncated || partial)
}
@available(macOS 14.0, *)
@MainActor
func screenshot(_ window: SCWindow, width: Int) async throws -> CGImage {
    let config = SCStreamConfiguration()
    config.width = width
    config.height = max(1, min(1000, Int(Double(width) * window.frame.height / max(1, window.frame.width))))
    config.showsCursor = false
    config.ignoreShadowsSingleWindow = true
    return try await SCScreenshotManager.captureImage(contentFilter: SCContentFilter(desktopIndependentWindow: window), configuration: config)
}
@MainActor
func previews(requestPermission: Bool) async {
    let granted = CGPreflightScreenCaptureAccess() || (requestPermission && CGRequestScreenCaptureAccess())
    guard granted else { emit(["permission": false, "windows": []]); return }
    guard #available(macOS 14.0, *) else { fail("version") }
    do {
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
        let ordered = (CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []).compactMap { $0[kCGWindowNumber as String] as? UInt32 }
        let rank = Dictionary(uniqueKeysWithValues: ordered.enumerated().map { ($0.element, $0.offset) })
        let windows = content.windows.filter {
            $0.windowLayer == 0 && $0.frame.width >= 200 && $0.frame.height >= 120 &&
            $0.owningApplication?.processID != getppid() &&
            !["local.me.desktop", "local.me.desktop.dev", "local.me.desktop.preview", "local.me.registration-test", "test.me.credential.preview"].contains($0.owningApplication?.bundleIdentifier ?? "")
        }
        .sorted { (rank[$0.windowID] ?? Int.max) < (rank[$1.windowID] ?? Int.max) }
        var result: [[String: Any]] = []
        for window in windows.prefix(12) {
            guard let app = window.owningApplication else { continue }
            var row: [String: Any] = ["id": window.windowID, "pid": app.processID, "bundle": app.bundleIdentifier, "name": app.applicationName, "title": window.title ?? app.applicationName]
            if let image = try? await screenshot(window, width: 480),
               let data = NSBitmapImageRep(cgImage: image).representation(using: .jpeg, properties: [.compressionFactor: 0.65]) {
                row["preview"] = data.base64EncodedString()
            }
            result.append(row)
        }
        emit(["permission": true, "windows": result, "more": windows.count > 12])
    } catch { fail("unavailable") }
}
@MainActor
func capture(_ source: [String: Any]) async {
    let (running, window) = selectedWindow(source)
    let (url, nodes, partial) = form(window)
    var output: [[String: String]] = []; var text = ""; var bytes = 0
    for node in nodes {
        if bytes >= 60000 { break }
        var row = ["label": node.label]
        text += node.label + "\n"
        if node.subrole != kAXSecureTextFieldSubrole {
            if node.role == kAXStaticTextRole || node.role == kAXHeadingRole || node.role == kAXButtonRole {
                row["text"] = String(string(node.element, kAXValueAttribute).prefix(4096))
                text += (row["text"] ?? "") + "\n"
            } else if semantic(node) != "password" && fields([node]).count == 1 {
                let allowed = ["api key","api token","access token","token","private key","public key","recovery codes","backup codes","network name","ssid","project","environment","permissions","scopes","host","hostname","port"]
                if semantic(node) != nil || allowed.contains(node.label.lowercased()) {
                    row["value"] = String(string(node.element, kAXValueAttribute).prefix(16384))
                }
            }
        }
        bytes += row.values.reduce(0) { $0 + $1.utf8.count }; output.append(row)
    }
    // OCR is local, selected-window only, and never supplies autofill targets.
    var ocr = ""
    if text.trimmingCharacters(in: .whitespacesAndNewlines).count < 40 && CGPreflightScreenCaptureAccess() {
        if #available(macOS 14.0, *), let content = try? await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true),
           let id = source["id"] as? UInt32, let selected = content.windows.first(where: {$0.windowID == id}),
           let image = try? await screenshot(selected, width: 1000) {
            let request = VNRecognizeTextRequest(); request.recognitionLevel = .accurate
            if (try? VNImageRequestHandler(cgImage: image).perform([request])) != nil {
                ocr = String((request.results ?? []).prefix(200).compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n").prefix(16000))
                text += ocr
            }
        }
    }
    let controls = fields(nodes)
    let passwords = controls.filter { semantic($0) == "password" }
    let users = controls.filter { semantic($0) == "username" }
    let lower = text.lowercased()
    let buttons = nodes.filter { $0.role == kAXButtonRole }.map { ($0.label.isEmpty ? string($0.element, kAXValueAttribute) : $0.label).lowercased().trimmingCharacters(in: .whitespacesAndNewlines) }
    let registering = registrationAction(buttons) && !lower.contains("current password") && !lower.contains("change password")
    let passwordPresent = passwords.contains { !string($0.element, kAXValueAttribute).isEmpty }
    let registration = registering && users.count == 1 && !passwords.isEmpty && passwords.count <= 2 && !passwordPresent
    var result: [String: Any] = ["source": running.localizedName ?? "Selected window", "title": string(window, kAXTitleAttribute), "url": url, "nodes": output, "ocr": ocr, "registration": registration, "password_present": passwordPresent, "truncated": partial || bytes >= 60000]
    let allowedBrowser = ["com.google.Chrome", "org.chromium.Chromium", "com.microsoft.edgemac", "com.brave.Browser", "com.apple.Safari", "org.mozilla.firefox"].contains(running.bundleIdentifier ?? "")
    if registration && allowedBrowser && !partial && bytes < 60000 && origin(url) != nil && fillableURL(url) {
        let matched = controls.filter { semantic($0) != nil }
        let names = matched.filter { semantic($0) == "full_name" }
        if names.count <= 1 && matched.allSatisfy({settable($0.element)}) {
            result["fill_target"] = ["source": source, "page": url, "fingerprint": fingerprint(controls), "name_required": names.contains { read($0.element, "AXRequired") as? Bool == true }]
        }
    }
    emit(result)
}
func fill(_ request: [String: Any]) {
    guard let target = request["target"] as? [String: Any], let source = target["source"] as? [String: Any],
          let page = target["page"] as? String, fillableURL(page), origin(page) != nil,
          let username = request["username"] as? String, !username.isEmpty,
          let password = request["password"] as? String, !password.isEmpty else { fail("invalid") }
    let (running, window) = selectedWindow(source)
    let (current, nodes, partial) = form(window)
    let controls = fields(nodes)
    guard current == page, !partial, fingerprint(controls) == target["fingerprint"] as? String else { fail("changed") }
    let matched = controls.filter { semantic($0) != nil }
    guard matched.filter({semantic($0) == "username"}).count == 1,
          (1...2).contains(matched.filter({semantic($0) == "password"}).count),
          matched.allSatisfy({settable($0.element)}) else { fail("unavailable") }
    var values: [(Node, String)] = []
    for node in matched {
        let key = semantic(node)!
        let value = key == "username" ? username : key == "password" ? password : (request["full_name"] as? String ?? "")
        if value.isEmpty {
            if read(node.element, "AXRequired") as? Bool == true { fail("required") }
            continue
        }
        let currentValue = string(node.element, kAXValueAttribute)
        // Never replace a password already in the page or a different account/name.
        guard currentValue.isEmpty || (key != "password" && currentValue == value) else { fail("occupied") }
        if key == "password" || currentValue != value { values.append((node, value)) }
    }
    for (node, value) in values {
        let (freshURL, freshNodes, freshPartial) = form(window)
        guard freshURL == page, !freshPartial, fingerprint(fields(freshNodes)) == target["fingerprint"] as? String else { fail("changed") }
        guard AXUIElementSetAttributeValue(node.element, kAXValueAttribute as CFString, value as CFTypeRef) == .success else { fail("partial") }
        // Browsers acknowledge the AX action before their renderer updates the value.
        let deadline = Date().addingTimeInterval(0.6)
        var applied = false
        repeat {
            let observed = string(node.element, kAXValueAttribute)
            applied = semantic(node) == "password" ? !observed.isEmpty : observed == value
            if !applied { Thread.sleep(forTimeInterval: 0.02) }
        } while !applied && Date() < deadline
        guard applied else { fail("partial") }
    }
    running.activate(options: [])
    AXUIElementPerformAction(window, kAXRaiseAction as CFString)
    emit(["filled": true])
}
func registrationAction(_ buttons: [String]) -> Bool {
    let positive = ["sign up", "signup", "register", "create account", "create an account"]
    let negative = ["sign in", "signin", "log in", "login", "change password", "reset password", "update password"]
    return buttons.contains { value in positive.contains { value == $0 || value.hasPrefix($0 + " ") } }
        && !buttons.contains { negative.contains($0) }
}
func fillableURL(_ value: String) -> Bool {
    guard origin(value) != nil, let url = URLComponents(string: value) else { return false }
    return url.scheme == "https" || (url.scheme == "http" && ["localhost", "127.0.0.1", "[::1]", "::1"].contains(url.host ?? ""))
}
let args = CommandLine.arguments
if args.count == 2 && args[1] == "self-test" {
    let dummy = AXUIElementCreateApplication(0)
    let email = Node(element: dummy, path: [1,2], role: kAXTextFieldRole, subrole: "", label: "Email address *")
    let password = Node(element: dummy, path: [1,3], role: kAXTextFieldRole, subrole: kAXSecureTextFieldSubrole, label: "Confirm password")
    assert(registrationAction(["register"]))
    assert(!registrationAction(["sign in"]))
    assert(!registrationAction(["sign in", "register"]))
    assert(semantic(email) == "username" && semantic(password) == "password")
    assert(fingerprint([email, password]) != fingerprint([password, email]))
    assert(origin("https://forge.laravel.com/register?token=example") == "https://forge.laravel.com")
    assert(origin("https://name:secret@forge.laravel.com") == nil)
    assert(origin("javascript:example") == nil)
    assert(fillableURL("http://127.0.0.1:18892/register"))
    assert(!fillableURL("http://example.test/register"))
    emit(["passed": true])
} else if args.count >= 2 && args[1] == "permissions" {
    if args.contains("screen") { _ = CGRequestScreenCaptureAccess() }
    if args.contains("accessibility") {
        _ = AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary)
    }
    emit(["screen_recording": CGPreflightScreenCaptureAccess(), "accessibility": trusted()])
} else if args.count >= 2 && args[1] == "accessibility" { accessibility(requestPermission: args.contains("request")) }
else if args.count == 2 && args[1] == "fill" { fill(input()) }
else if args.count == 2 && args[1] == "capture" {
    let source = input()
    let application = NSApplication.shared; application.setActivationPolicy(.prohibited)
    Task { @MainActor in await capture(source); exit(0) }; application.run()
} else if args.count >= 2 && args[1] == "windows" {
    let application = NSApplication.shared; application.setActivationPolicy(.prohibited)
    Task { @MainActor in await previews(requestPermission: args.contains("request")); exit(0) }; application.run()
} else { fail("invalid") }
