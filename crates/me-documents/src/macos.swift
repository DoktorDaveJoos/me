// Built and embedded by me-documents. Input/output are pipes, never document files.
import Foundation
import AppKit
import PDFKit
import Vision
import ImageIO

struct Part: Codable {
    let text: String
    let page: Int?
    let section: String?
    let method: String
}
enum Failure: Int32, Error { case invalid = 2, locked = 3, limit = 4, empty = 5, ocr = 6 }
let maxText = 8 * 1_048_576
let maxPages = 500
func progress(_ page: Int, _ total: Int) throws {
    try FileHandle.standardError.write(contentsOf: Data("ME_PAGE \(page) \(total)\n".utf8))
}
var parts = [Part]()
var textSize = 0
func add(_ text: String, _ page: Int?, _ method: String) throws {
    guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
    textSize += text.utf8.count
    guard textSize <= maxText else { throw Failure.limit }
    parts.append(Part(text: text, page: page, section: nil, method: method))
}
func recognize(_ image: CGImage) throws -> String {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.recognitionLanguages = ["de-DE", "en-US"]
    // Avoid dictionary corrections of names and identity numbers.
    request.usesLanguageCorrection = false
    do { try VNImageRequestHandler(cgImage: image, options: [:]).perform([request]) }
    catch { throw Failure.ocr }
    // Vision may return column blocks out of reading order. Reconstruct visual
    // rows so a payroll label stays beside its amount instead of a distant column.
    let observations = (request.results ?? []).sorted { $0.boundingBox.midY > $1.boundingBox.midY }
    var rows = [[VNRecognizedTextObservation]]()
    for observation in observations {
        if let last = rows.last, let anchor = last.first,
           abs(anchor.boundingBox.midY - observation.boundingBox.midY) <= min(anchor.boundingBox.height, observation.boundingBox.height) * 0.45 {
            rows[rows.count - 1].append(observation)
        } else { rows.append([observation]) }
    }
    return rows.map { row in
        row.sorted { $0.boundingBox.minX < $1.boundingBox.minX }
            .compactMap { $0.topCandidates(1).first?.string }.joined(separator: "   ")
    }.joined(separator: "\n")
}
func key(_ text: String) -> String {
    text.lowercased().filter { !$0.isWhitespace }
}
do {
    guard CommandLine.arguments.count == 2 else { throw Failure.invalid }
    let ext = CommandLine.arguments[1]
    var data = Data()
    while let chunk = try FileHandle.standardInput.read(upToCount: 65536), !chunk.isEmpty {
        data.append(chunk)
        guard data.count <= 64 * 1024 * 1024 else { throw Failure.limit }
    }
    if ext == "pdf" {
        guard data.starts(with: Data("%PDF-".utf8)), let pdf = PDFDocument(data: data) else { throw Failure.invalid }
        guard !pdf.isLocked else { throw Failure.locked }
        guard pdf.pageCount > 0, pdf.pageCount <= maxPages else { throw Failure.limit }
        try progress(0, pdf.pageCount)
        for index in 0..<pdf.pageCount {
            try autoreleasepool {
                guard let page = pdf.page(at: index) else { throw Failure.invalid }
                let widgets = page.annotations.compactMap { annotation -> String? in
                    guard let name = annotation.fieldName, !name.isEmpty, !annotation.isReadOnly else { return nil }
                    let value = annotation.widgetStringValue ?? ""
                    return value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || value == "Off"
                        ? "Empty form field: \(name)"
                        : "Filled form field: \(name): \(value)"
                }.joined(separator: "\n")
                let text = [page.string ?? "", widgets].filter { !$0.isEmpty }.joined(separator: "\n")
                try add(text, index + 1, "pdf_text")
                // OCR also covers embedded scans on pages with an existing text layer.
                let bounds = page.bounds(for: .mediaBox)
                guard bounds.width.isFinite, bounds.height.isFinite, bounds.width > 0, bounds.height > 0 else { throw Failure.invalid }
                let scale = min(3.0, 3200 / max(bounds.width, bounds.height))
                let preview = page.thumbnail(of: NSSize(width: bounds.width * scale, height: bounds.height * scale), for: .mediaBox)
                guard let cg = preview.cgImage(forProposedRect: nil, context: nil, hints: nil) else { throw Failure.invalid }
                let recognized = try recognize(cg)
                let existing = key(text)
                let additional = recognized.components(separatedBy: .newlines).filter { !key($0).isEmpty && !existing.contains(key($0)) }.joined(separator: "\n")
                try add(additional, index + 1, "ocr")
                try progress(index + 1, pdf.pageCount)
            }
        }
    } else if ext == "rtf" {
        guard data.starts(with: Data("{\\rtf".utf8)) else { throw Failure.invalid }
        let attributed = try NSAttributedString(data: data, options: [.documentType: NSAttributedString.DocumentType.rtf], documentAttributes: nil)
        try add(attributed.string, nil, "office")
    } else {
        guard let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary) else { throw Failure.invalid }
        let count = CGImageSourceGetCount(source)
        guard count > 0, count <= maxPages else { throw Failure.limit }
        try progress(0, count)
        for index in 0..<count {
            try autoreleasepool {
                guard let props = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any],
                      let width = props[kCGImagePropertyPixelWidth] as? NSNumber,
                      let height = props[kCGImagePropertyPixelHeight] as? NSNumber,
                      width.doubleValue > 0, height.doubleValue > 0,
                      width.doubleValue * height.doubleValue <= 100_000_000 else { throw Failure.limit }
                let options: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: 3200, kCGImageSourceCreateThumbnailWithTransform: true]
                guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, options as CFDictionary) else { throw Failure.invalid }
                try add(try recognize(image), index + 1, "ocr")
                try progress(index + 1, count)
            }
        }
    }
    guard !parts.isEmpty else { throw Failure.empty }
    try FileHandle.standardOutput.write(contentsOf: JSONEncoder().encode(parts))
} catch let failure as Failure { exit(failure.rawValue) }
catch { exit(Failure.invalid.rawValue) }
